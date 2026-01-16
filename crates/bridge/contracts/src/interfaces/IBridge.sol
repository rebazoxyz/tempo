// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IBridge
/// @notice Interface for the IBC-inspired bridge router
/// @dev Handles packet lifecycle: send → recv → ack
interface IBridge {
    /// @notice Packet structure for cross-chain messages
    struct Packet {
        /// @dev Unique sequence number for this packet on the source chain
        uint64 sequence;
        /// @dev Client ID on the source chain
        string sourceClient;
        /// @dev Client ID on the destination chain
        string destClient;
        /// @dev Timeout timestamp (nanoseconds since epoch)
        uint64 timeout;
        /// @dev Opaque payload data
        bytes payload;
    }

    /// @notice Emitted when a packet is sent
    event PacketSent(
        uint64 indexed sequence,
        string sourceClient,
        string destClient,
        uint64 timeout,
        bytes payload
    );

    /// @notice Emitted when a packet is received
    event PacketReceived(
        uint64 indexed sequence,
        string sourceClient,
        string destClient,
        bytes payload
    );

    /// @notice Emitted when a packet acknowledgement is processed
    event PacketAcknowledged(
        uint64 indexed sequence,
        string sourceClient,
        string destClient,
        bytes acknowledgement
    );

    /// @notice Emitted when a packet times out
    event PacketTimedOut(uint64 indexed sequence, string sourceClient, string destClient);

    /// @notice Error when packet has already been received
    error PacketAlreadyReceived(uint64 sequence);

    /// @notice Error when packet has already been acknowledged
    error PacketAlreadyAcknowledged(uint64 sequence);

    /// @notice Error when packet has timed out
    error PacketTimeout(uint64 sequence, uint64 timeout, uint64 currentTime);

    /// @notice Error when proof verification fails
    error InvalidProof();

    /// @notice Error when light client is not registered
    error ClientNotFound(string clientId);

    /// @notice Error when commitment does not match
    error CommitmentMismatch(bytes32 expected, bytes32 actual);

    /// @notice Send a packet to a destination chain
    /// @param destClient The client ID for the destination chain
    /// @param payload The packet payload
    /// @param timeout Timeout timestamp in nanoseconds
    /// @return sequence The sequence number assigned to this packet
    function sendPacket(
        string calldata destClient,
        bytes calldata payload,
        uint64 timeout
    ) external returns (uint64 sequence);

    /// @notice Receive a packet from a source chain
    /// @param packet The packet to receive
    /// @param proof Membership proof for the packet commitment
    /// @param proofHeight Height at which the proof was generated
    function recvPacket(
        Packet calldata packet,
        bytes calldata proof,
        uint64 proofHeight
    ) external;

    /// @notice Process an acknowledgement for a previously sent packet
    /// @param packet The original packet that was sent
    /// @param ack The acknowledgement data
    /// @param proof Membership proof for the acknowledgement
    /// @param proofHeight Height at which the proof was generated
    function acknowledgePacket(
        Packet calldata packet,
        bytes calldata ack,
        bytes calldata proof,
        uint64 proofHeight
    ) external;

    /// @notice Timeout a packet that was not received before deadline
    /// @param packet The packet to timeout
    /// @param proof Non-membership proof that packet was not received
    /// @param proofHeight Height at which the proof was generated
    function timeoutPacket(
        Packet calldata packet,
        bytes calldata proof,
        uint64 proofHeight
    ) external;

    /// @notice Get the next sequence number for a client
    /// @param clientId The client ID
    /// @return sequence The next sequence number
    function getNextSequence(string calldata clientId) external view returns (uint64 sequence);

    /// @notice Get the packet commitment for a sent packet
    /// @param clientId The client ID
    /// @param sequence The sequence number
    /// @return commitment The packet commitment hash
    function getPacketCommitment(
        string calldata clientId,
        uint64 sequence
    ) external view returns (bytes32 commitment);

    /// @notice Check if a packet has been received
    /// @param clientId The client ID
    /// @param sequence The sequence number
    /// @return received True if the packet was received
    function hasPacketReceipt(
        string calldata clientId,
        uint64 sequence
    ) external view returns (bool received);
}
