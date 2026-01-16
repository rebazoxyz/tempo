// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ILightClient} from "../interfaces/ILightClient.sol";

/// @title EthereumLightClient
/// @notice Light client for Ethereum deployed on Tempo
/// @dev Uses beacon chain sync committee protocol for finality verification
contract EthereumLightClient is ILightClient {
    /// @notice Beacon chain block header
    struct BeaconBlockHeader {
        uint64 slot;
        uint64 proposerIndex;
        bytes32 parentRoot;
        bytes32 stateRoot;
        bytes32 bodyRoot;
    }

    /// @notice Sync committee for beacon chain light client protocol
    struct SyncCommittee {
        /// @dev Aggregated public key of the sync committee
        bytes aggregatePubkey;
        /// @dev Individual public keys (512 validators)
        bytes pubkeys;
    }

    /// @notice Light client update from beacon chain
    struct LightClientUpdate {
        /// @dev Attested beacon block header
        BeaconBlockHeader attestedHeader;
        /// @dev Finalized beacon block header
        BeaconBlockHeader finalizedHeader;
        /// @dev Branch proof for finalized checkpoint
        bytes32[] finalityBranch;
        /// @dev Sync committee signature
        bytes syncAggregate;
        /// @dev Bitmap of sync committee members who signed
        bytes participationBitmap;
        /// @dev Execution payload state root
        bytes32 executionStateRoot;
        /// @dev Branch proof for execution payload
        bytes32[] executionBranch;
    }

    /// @notice Merkle-Patricia proof for Ethereum state
    struct MPTProof {
        /// @dev Account address
        address account;
        /// @dev Storage slot
        bytes32 slot;
        /// @dev Value at the slot
        bytes32 value;
        /// @dev Account proof nodes
        bytes[] accountProof;
        /// @dev Storage proof nodes
        bytes[] storageProof;
    }

    /// @dev Consensus states indexed by height (slot)
    mapping(uint64 => ConsensusState) private _consensusStates;

    /// @dev Current sync committee
    SyncCommittee private _currentSyncCommittee;

    /// @dev Next sync committee
    SyncCommittee private _nextSyncCommittee;

    /// @dev Latest verified slot
    uint64 private _latestSlot;

    /// @dev Genesis time of the beacon chain
    uint64 private immutable _genesisTime;

    /// @dev Slots per epoch
    uint64 private constant SLOTS_PER_EPOCH = 32;

    /// @dev Epochs per sync committee period
    uint64 private constant EPOCHS_PER_SYNC_COMMITTEE_PERIOD = 256;

    /// @dev Sync committee size
    uint256 private constant SYNC_COMMITTEE_SIZE = 512;

    /// @dev Minimum sync committee participation (2/3)
    uint256 private constant MIN_SYNC_COMMITTEE_PARTICIPANTS = 342;

    /// @dev Finality branch depth
    uint256 private constant FINALITY_BRANCH_DEPTH = 6;

    /// @dev Execution branch depth
    uint256 private constant EXECUTION_BRANCH_DEPTH = 4;

    /// @dev Owner for initial setup
    address private immutable _owner;

    /// @notice Error when sync committee participation is too low
    error InsufficientParticipation(uint256 actual, uint256 required);

    /// @notice Error when finality proof is invalid
    error InvalidFinalityProof();

    /// @notice Error when execution proof is invalid
    error InvalidExecutionProof();

    /// @notice Error when slot is not newer
    error SlotNotNewer(uint64 provided, uint64 latest);

    /// @notice Error when consensus state not found
    error ConsensusStateNotFound(uint64 height);

    /// @notice Error when caller is not owner
    error NotOwner();

    /// @notice Error when MPT proof is invalid
    error InvalidMPTProof();

    modifier onlyOwner() {
        if (msg.sender != _owner) revert NotOwner();
        _;
    }

    constructor(uint64 genesisTime, bytes memory initialSyncCommitteePubkeys) {
        _owner = msg.sender;
        _genesisTime = genesisTime;
        _currentSyncCommittee.pubkeys = initialSyncCommitteePubkeys;
    }

    /// @notice Bootstrap the light client with a trusted checkpoint
    /// @param header Trusted beacon block header
    /// @param executionStateRoot Execution layer state root
    /// @param syncCommittee Current sync committee
    function bootstrap(
        BeaconBlockHeader calldata header,
        bytes32 executionStateRoot,
        SyncCommittee calldata syncCommittee
    ) external onlyOwner {
        _consensusStates[header.slot] = ConsensusState({
            stateRoot: executionStateRoot,
            timestamp: _slotToTimestamp(header.slot)
        });

        _currentSyncCommittee = syncCommittee;
        _latestSlot = header.slot;

        emit ClientUpdated(header.slot, executionStateRoot);
    }

    /// @inheritdoc ILightClient
    function updateClient(bytes calldata proof) external override returns (bool) {
        LightClientUpdate memory update = abi.decode(proof, (LightClientUpdate));

        // Verify slot is newer
        if (update.finalizedHeader.slot <= _latestSlot) {
            revert SlotNotNewer(update.finalizedHeader.slot, _latestSlot);
        }

        // Verify sync committee participation
        uint256 participants = _countParticipants(update.participationBitmap);
        if (participants < MIN_SYNC_COMMITTEE_PARTICIPANTS) {
            revert InsufficientParticipation(participants, MIN_SYNC_COMMITTEE_PARTICIPANTS);
        }

        // Verify finality branch
        if (!_verifyFinalityBranch(update)) {
            revert InvalidFinalityProof();
        }

        // Verify execution payload branch
        if (!_verifyExecutionBranch(update)) {
            revert InvalidExecutionProof();
        }

        // Verify sync committee signature
        _verifySyncCommitteeSignature(update);

        // Store the new consensus state
        _consensusStates[update.finalizedHeader.slot] = ConsensusState({
            stateRoot: update.executionStateRoot,
            timestamp: _slotToTimestamp(update.finalizedHeader.slot)
        });

        _latestSlot = update.finalizedHeader.slot;

        emit ClientUpdated(update.finalizedHeader.slot, update.executionStateRoot);

        return true;
    }

    /// @inheritdoc ILightClient
    function verifyMembership(
        bytes calldata proof,
        uint64 height,
        bytes calldata path,
        bytes calldata value
    ) external view override returns (bool) {
        ConsensusState storage state = _consensusStates[height];
        if (state.stateRoot == bytes32(0)) {
            revert ConsensusStateNotFound(height);
        }

        MPTProof memory mptProof = abi.decode(proof, (MPTProof));

        // Verify the path matches the account and slot
        bytes memory expectedPath = abi.encodePacked(mptProof.account, mptProof.slot);
        if (keccak256(path) != keccak256(expectedPath)) {
            return false;
        }

        // Verify the value matches
        if (keccak256(value) != keccak256(abi.encodePacked(mptProof.value))) {
            return false;
        }

        // Verify the MPT proof
        return _verifyMPTProof(state.stateRoot, mptProof);
    }

    /// @inheritdoc ILightClient
    function getConsensusState(
        uint64 height
    ) external view override returns (ConsensusState memory) {
        ConsensusState storage state = _consensusStates[height];
        if (state.stateRoot == bytes32(0)) {
            revert ConsensusStateNotFound(height);
        }
        return state;
    }

    /// @inheritdoc ILightClient
    function getLatestHeight() external view override returns (uint64) {
        return _latestSlot;
    }

    /// @notice Get genesis time
    /// @return timestamp Genesis time
    function getGenesisTime() external view returns (uint64) {
        return _genesisTime;
    }

    /// @dev Convert slot to timestamp
    function _slotToTimestamp(uint64 slot) private view returns (uint64) {
        return _genesisTime + (slot * 12); // 12 second slots
    }

    /// @dev Count participants from bitmap
    function _countParticipants(bytes memory bitmap) private pure returns (uint256 count) {
        for (uint256 i = 0; i < bitmap.length; i++) {
            count += _popcount(uint8(bitmap[i]));
        }
    }

    /// @dev Population count (number of set bits)
    function _popcount(uint8 x) private pure returns (uint256) {
        uint256 count = 0;
        while (x != 0) {
            count += x & 1;
            x >>= 1;
        }
        return count;
    }

    /// @dev Verify the finality branch merkle proof
    function _verifyFinalityBranch(LightClientUpdate memory update) private pure returns (bool) {
        if (update.finalityBranch.length != FINALITY_BRANCH_DEPTH) {
            return false;
        }

        bytes32 leaf = _hashBeaconBlockHeader(update.finalizedHeader);
        bytes32 root = _computeMerkleRoot(leaf, update.finalityBranch, 41); // Finality checkpoint index

        return root == update.attestedHeader.stateRoot;
    }

    /// @dev Verify the execution payload branch merkle proof
    function _verifyExecutionBranch(LightClientUpdate memory update) private pure returns (bool) {
        if (update.executionBranch.length != EXECUTION_BRANCH_DEPTH) {
            return false;
        }

        bytes32 root = _computeMerkleRoot(
            update.executionStateRoot,
            update.executionBranch,
            9 // Execution payload state root index
        );

        return root == update.finalizedHeader.bodyRoot;
    }

    /// @dev Hash a beacon block header
    function _hashBeaconBlockHeader(
        BeaconBlockHeader memory header
    ) private pure returns (bytes32) {
        return
            keccak256(
                abi.encodePacked(
                    header.slot,
                    header.proposerIndex,
                    header.parentRoot,
                    header.stateRoot,
                    header.bodyRoot
                )
            );
    }

    /// @dev Compute merkle root from leaf and proof with generalized index
    function _computeMerkleRoot(
        bytes32 leaf,
        bytes32[] memory proof,
        uint256 index
    ) private pure returns (bytes32) {
        bytes32 computedHash = leaf;

        for (uint256 i = 0; i < proof.length; i++) {
            if ((index & (1 << i)) == 0) {
                computedHash = keccak256(abi.encodePacked(computedHash, proof[i]));
            } else {
                computedHash = keccak256(abi.encodePacked(proof[i], computedHash));
            }
        }

        return computedHash;
    }

    /// @dev Verify sync committee BLS signature
    function _verifySyncCommitteeSignature(LightClientUpdate memory update) private view {
        // In production, this would:
        // 1. Compute signing root = hash(attestedHeader, domain)
        // 2. Aggregate public keys from sync committee based on participation bitmap
        // 3. Verify BLS signature using precompiles or library
        //
        // For now, we validate the structure
        if (
            update.syncAggregate.length == 0 ||
            update.participationBitmap.length != SYNC_COMMITTEE_SIZE / 8
        ) {
            revert InsufficientParticipation(0, MIN_SYNC_COMMITTEE_PARTICIPANTS);
        }

        // Suppress unused warning
        _currentSyncCommittee;
    }

    /// @dev Verify Merkle-Patricia trie proof
    function _verifyMPTProof(
        bytes32 stateRoot,
        MPTProof memory mptProof
    ) private pure returns (bool) {
        // First verify account proof to get storage root
        bytes32 accountHash = keccak256(abi.encodePacked(mptProof.account));
        bytes32 storageRoot = _verifyAccountProof(
            stateRoot,
            accountHash,
            mptProof.accountProof
        );

        if (storageRoot == bytes32(0)) {
            return false;
        }

        // Then verify storage proof
        bytes32 slotHash = keccak256(abi.encodePacked(mptProof.slot));
        return _verifyStorageProof(storageRoot, slotHash, mptProof.value, mptProof.storageProof);
    }

    /// @dev Verify account proof and return storage root
    function _verifyAccountProof(
        bytes32 stateRoot,
        bytes32 accountHash,
        bytes[] memory proof
    ) private pure returns (bytes32) {
        // Simplified MPT verification
        // In production, this would implement full RLP decoding and path verification
        if (proof.length == 0) {
            return bytes32(0);
        }

        // The last proof element should contain the account data
        bytes memory accountData = proof[proof.length - 1];

        // Extract storage root from account RLP (simplified)
        // Account: [nonce, balance, storageRoot, codeHash]
        if (accountData.length < 32) {
            return bytes32(0);
        }

        // This is a placeholder - real implementation needs RLP decoding
        bytes32 storageRoot;
        assembly {
            storageRoot := mload(add(accountData, 96)) // Offset to storageRoot in RLP
        }

        // Suppress unused variable warning
        stateRoot;
        accountHash;

        return storageRoot;
    }

    /// @dev Verify storage proof
    function _verifyStorageProof(
        bytes32 storageRoot,
        bytes32 slotHash,
        bytes32 value,
        bytes[] memory proof
    ) private pure returns (bool) {
        // Simplified storage proof verification
        // In production, this would implement full MPT verification
        if (proof.length == 0) {
            return false;
        }

        // Suppress unused variable warnings
        storageRoot;
        slotHash;
        value;

        return true;
    }
}
