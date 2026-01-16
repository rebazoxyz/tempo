// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IBridge} from "./interfaces/IBridge.sol";
import {ILightClient} from "./interfaces/ILightClient.sol";

/// @title Bridge
/// @notice IBC-inspired bridge router for cross-chain packet relay
/// @dev Manages light clients and handles packet lifecycle (send → recv → ack)
contract Bridge is IBridge {
    /// @dev Registered light clients by client ID
    mapping(string => ILightClient) private _lightClients;

    /// @dev Next sequence number for each client
    mapping(string => uint64) private _nextSequenceSend;

    /// @dev Packet commitments: clientId => sequence => commitment
    mapping(string => mapping(uint64 => bytes32)) private _packetCommitments;

    /// @dev Packet receipts: clientId => sequence => received
    mapping(string => mapping(uint64 => bool)) private _packetReceipts;

    /// @dev Packet acknowledgements: clientId => sequence => acknowledgement
    mapping(string => mapping(uint64 => bytes)) private _packetAcks;

    /// @dev Owner for client registration
    address private immutable _owner;

    /// @dev Commitment path prefix
    bytes private constant COMMITMENT_PREFIX = "commitments/";

    /// @dev Receipt path prefix
    bytes private constant RECEIPT_PREFIX = "receipts/";

    /// @dev Acknowledgement path prefix
    bytes private constant ACK_PREFIX = "acks/";

    /// @notice Error when caller is not owner
    error NotOwner();

    /// @notice Error when client already exists
    error ClientAlreadyExists(string clientId);

    modifier onlyOwner() {
        if (msg.sender != _owner) revert NotOwner();
        _;
    }

    constructor() {
        _owner = msg.sender;
    }

    /// @notice Register a light client
    /// @param clientId Unique identifier for the client
    /// @param lightClient Address of the light client contract
    function registerClient(
        string calldata clientId,
        ILightClient lightClient
    ) external onlyOwner {
        if (address(_lightClients[clientId]) != address(0)) {
            revert ClientAlreadyExists(clientId);
        }
        _lightClients[clientId] = lightClient;
        _nextSequenceSend[clientId] = 1;
    }

    /// @inheritdoc IBridge
    function sendPacket(
        string calldata destClient,
        bytes calldata payload,
        uint64 timeout
    ) external override returns (uint64 sequence) {
        if (address(_lightClients[destClient]) == address(0)) {
            revert ClientNotFound(destClient);
        }

        sequence = _nextSequenceSend[destClient];
        _nextSequenceSend[destClient] = sequence + 1;

        // Create packet commitment
        bytes32 commitment = keccak256(
            abi.encodePacked(sequence, destClient, timeout, keccak256(payload))
        );

        _packetCommitments[destClient][sequence] = commitment;

        emit PacketSent(sequence, _getSourceClientId(), destClient, timeout, payload);

        return sequence;
    }

    /// @inheritdoc IBridge
    function recvPacket(
        Packet calldata packet,
        bytes calldata proof,
        uint64 proofHeight
    ) external override {
        ILightClient lightClient = _lightClients[packet.sourceClient];
        if (address(lightClient) == address(0)) {
            revert ClientNotFound(packet.sourceClient);
        }

        // Check if already received
        if (_packetReceipts[packet.sourceClient][packet.sequence]) {
            revert PacketAlreadyReceived(packet.sequence);
        }

        // Check timeout
        ILightClient.ConsensusState memory consensusState = lightClient.getConsensusState(
            proofHeight
        );
        if (packet.timeout != 0 && consensusState.timestamp >= packet.timeout) {
            revert PacketTimeout(packet.sequence, packet.timeout, consensusState.timestamp);
        }

        // Verify packet commitment on source chain
        bytes memory path = _packetCommitmentPath(packet.sourceClient, packet.sequence);
        bytes32 expectedCommitment = keccak256(
            abi.encodePacked(
                packet.sequence,
                packet.destClient,
                packet.timeout,
                keccak256(packet.payload)
            )
        );

        bool valid = lightClient.verifyMembership(
            proof,
            proofHeight,
            path,
            abi.encodePacked(expectedCommitment)
        );

        if (!valid) {
            revert InvalidProof();
        }

        // Mark as received
        _packetReceipts[packet.sourceClient][packet.sequence] = true;

        emit PacketReceived(packet.sequence, packet.sourceClient, packet.destClient, packet.payload);
    }

    /// @inheritdoc IBridge
    function acknowledgePacket(
        Packet calldata packet,
        bytes calldata ack,
        bytes calldata proof,
        uint64 proofHeight
    ) external override {
        ILightClient lightClient = _lightClients[packet.destClient];
        if (address(lightClient) == address(0)) {
            revert ClientNotFound(packet.destClient);
        }

        // Verify we sent this packet
        bytes32 commitment = _packetCommitments[packet.destClient][packet.sequence];
        if (commitment == bytes32(0)) {
            revert PacketAlreadyAcknowledged(packet.sequence);
        }

        bytes32 expectedCommitment = keccak256(
            abi.encodePacked(
                packet.sequence,
                packet.destClient,
                packet.timeout,
                keccak256(packet.payload)
            )
        );

        if (commitment != expectedCommitment) {
            revert CommitmentMismatch(expectedCommitment, commitment);
        }

        // Verify acknowledgement on destination chain
        bytes memory path = _packetAckPath(packet.destClient, packet.sequence);

        bool valid = lightClient.verifyMembership(proof, proofHeight, path, ack);

        if (!valid) {
            revert InvalidProof();
        }

        // Delete commitment (packet lifecycle complete)
        delete _packetCommitments[packet.destClient][packet.sequence];

        emit PacketAcknowledged(packet.sequence, packet.sourceClient, packet.destClient, ack);
    }

    /// @inheritdoc IBridge
    function timeoutPacket(
        Packet calldata packet,
        bytes calldata proof,
        uint64 proofHeight
    ) external override {
        ILightClient lightClient = _lightClients[packet.destClient];
        if (address(lightClient) == address(0)) {
            revert ClientNotFound(packet.destClient);
        }

        // Verify we sent this packet
        bytes32 commitment = _packetCommitments[packet.destClient][packet.sequence];
        if (commitment == bytes32(0)) {
            revert PacketAlreadyAcknowledged(packet.sequence);
        }

        // Verify timeout has passed on destination chain
        ILightClient.ConsensusState memory consensusState = lightClient.getConsensusState(
            proofHeight
        );

        if (consensusState.timestamp < packet.timeout) {
            revert PacketTimeout(packet.sequence, packet.timeout, consensusState.timestamp);
        }

        // Verify non-membership of receipt on destination chain
        bytes memory path = _packetReceiptPath(packet.destClient, packet.sequence);

        // For timeout, we verify that the receipt does NOT exist
        // This is a non-membership proof - the proof should show the value is empty
        bool valid = lightClient.verifyMembership(proof, proofHeight, path, "");

        if (!valid) {
            revert InvalidProof();
        }

        // Delete commitment
        delete _packetCommitments[packet.destClient][packet.sequence];

        emit PacketTimedOut(packet.sequence, packet.sourceClient, packet.destClient);
    }

    /// @notice Write acknowledgement for a received packet
    /// @param packet The packet being acknowledged
    /// @param ack The acknowledgement data
    function writeAcknowledgement(Packet calldata packet, bytes calldata ack) external {
        // Only callable after packet is received
        if (!_packetReceipts[packet.sourceClient][packet.sequence]) {
            revert PacketAlreadyReceived(packet.sequence);
        }

        // Only write once
        if (_packetAcks[packet.sourceClient][packet.sequence].length > 0) {
            revert PacketAlreadyAcknowledged(packet.sequence);
        }

        _packetAcks[packet.sourceClient][packet.sequence] = ack;
    }

    /// @inheritdoc IBridge
    function getNextSequence(string calldata clientId) external view override returns (uint64) {
        return _nextSequenceSend[clientId];
    }

    /// @inheritdoc IBridge
    function getPacketCommitment(
        string calldata clientId,
        uint64 sequence
    ) external view override returns (bytes32) {
        return _packetCommitments[clientId][sequence];
    }

    /// @inheritdoc IBridge
    function hasPacketReceipt(
        string calldata clientId,
        uint64 sequence
    ) external view override returns (bool) {
        return _packetReceipts[clientId][sequence];
    }

    /// @notice Get the acknowledgement for a packet
    /// @param clientId The client ID
    /// @param sequence The sequence number
    /// @return ack The acknowledgement data
    function getPacketAcknowledgement(
        string calldata clientId,
        uint64 sequence
    ) external view returns (bytes memory) {
        return _packetAcks[clientId][sequence];
    }

    /// @notice Get the light client for a client ID
    /// @param clientId The client ID
    /// @return lightClient The light client address
    function getLightClient(string calldata clientId) external view returns (ILightClient) {
        return _lightClients[clientId];
    }

    /// @dev Get the source client ID (this chain's identifier)
    function _getSourceClientId() private pure returns (string memory) {
        // In production, this would be configurable
        return "tempo-mainnet";
    }

    /// @dev Build packet commitment path
    function _packetCommitmentPath(
        string memory clientId,
        uint64 sequence
    ) private pure returns (bytes memory) {
        return abi.encodePacked(COMMITMENT_PREFIX, clientId, "/", sequence);
    }

    /// @dev Build packet receipt path
    function _packetReceiptPath(
        string memory clientId,
        uint64 sequence
    ) private pure returns (bytes memory) {
        return abi.encodePacked(RECEIPT_PREFIX, clientId, "/", sequence);
    }

    /// @dev Build packet acknowledgement path
    function _packetAckPath(
        string memory clientId,
        uint64 sequence
    ) private pure returns (bytes memory) {
        return abi.encodePacked(ACK_PREFIX, clientId, "/", sequence);
    }
}
