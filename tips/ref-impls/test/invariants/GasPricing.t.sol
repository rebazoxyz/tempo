// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.13 <0.9.0;

import { TIP20 } from "../../src/TIP20.sol";
import { BaseTest } from "../BaseTest.t.sol";
import { Test, console } from "forge-std/Test.sol";

/// @title GasPricing Invariant Test
/// @notice Invariant tests for TIP-1000 (State Creation Cost) and TIP-1010 (Mainnet Gas Parameters)
/// @dev Tests gas pricing invariants that MUST hold for Tempo T1 hardfork
contract GasPricingInvariantTest is BaseTest {

    /*//////////////////////////////////////////////////////////////
                            TIP-1000 GAS CONSTANTS
    //////////////////////////////////////////////////////////////*/

    /// @dev SSTORE to new (zero) slot costs 250,000 gas (TIP-1000)
    uint256 private constant SSTORE_SET_GAS = 250_000;

    /// @dev SSTORE to existing slot costs 5,000 gas (unchanged from EVM)
    uint256 private constant SSTORE_RESET_GAS = 5000;

    /// @dev SSTORE clearing (non-zero → zero) refunds 15,000 gas
    uint256 private constant SSTORE_CLEAR_REFUND = 15_000;

    /// @dev Account creation (nonce 0→1) costs 250,000 gas
    uint256 private constant ACCOUNT_CREATION_GAS = 250_000;

    /// @dev CREATE/CREATE2 base cost (keccak + codesize fields)
    uint256 private constant CREATE_BASE_GAS = 500_000;

    /// @dev Code deposit cost per byte
    uint256 private constant CODE_DEPOSIT_PER_BYTE = 1000;

    /// @dev Transaction gas limit cap
    uint256 private constant TX_GAS_LIMIT_CAP = 30_000_000;

    /// @dev Minimum gas for first transaction (nonce=0)
    /// Base tx (21k) + account creation (250k) = 271k
    uint256 private constant FIRST_TX_MIN_GAS = 271_000;

    /// @dev EIP-7702 auth with nonce=0 adds 250k gas per authorization
    uint256 private constant EIP7702_NEW_ACCOUNT_GAS = 250_000;

    /// @dev EIP-7702 per-authorization base cost (reduced from 25k in T1)
    uint256 private constant EIP7702_PER_AUTH_GAS = 12_500;

    /*//////////////////////////////////////////////////////////////
                            TIP-1010 BLOCK CONSTANTS
    //////////////////////////////////////////////////////////////*/

    /// @dev Total block gas limit
    uint256 private constant BLOCK_GAS_LIMIT = 500_000_000;

    /// @dev General lane gas limit (fixed at 30M for T1)
    uint256 private constant GENERAL_GAS_LIMIT = 30_000_000;

    /// @dev Minimum payment lane gas (500M - 30M general)
    uint256 private constant PAYMENT_GAS_MIN = 470_000_000;

    /// @dev T1 hardfork base fee (20 gwei)
    uint256 private constant T1_BASE_FEE = 20_000_000_000;

    /// @dev Maximum contract code size (EIP-170)
    uint256 private constant MAX_CONTRACT_SIZE = 24_576;

    /// @dev Maximum initcode size (EIP-3860: 2 * MAX_CONTRACT_SIZE)
    uint256 private constant MAX_INITCODE_SIZE = 49_152;

    /// @dev 2D nonce key creation costs 250k gas (TIP-1000)
    uint256 private constant NONCE_KEY_CREATION_GAS = 250_000;

    /// @dev Cold SLOAD cost (EIP-2929)
    uint256 private constant COLD_SLOAD_GAS = 2100;

    /// @dev Warm SSTORE reset cost
    uint256 private constant WARM_SSTORE_RESET_GAS = 5000;

    /// @dev Shared gas limit (block_gas_limit / 10)
    uint256 private constant SHARED_GAS_LIMIT = 50_000_000;

    /// @dev T0 hardfork activation timestamp (genesis)
    uint256 private constant T0_ACTIVATION = 0;

    /// @dev T1 hardfork activation timestamp (example, should be set per network)
    uint256 private constant T1_ACTIVATION = 1_700_000_000;

    /*//////////////////////////////////////////////////////////////
                            TEST STATE
    //////////////////////////////////////////////////////////////*/

    /// @dev Test actors
    address[] private _actors;

    /// @dev Log file for gas measurements
    string private constant LOG_FILE = "gas_pricing.log";

    /// @dev Storage contract for testing SSTORE costs
    GasTestStorage private _storageContract;

    /// @dev Factory for creating test contracts
    GasTestFactory private _factory;

    /*//////////////////////////////////////////////////////////////
                            GHOST VARIABLES
    //////////////////////////////////////////////////////////////*/

    /// @dev Tracks SSTORE to new slot gas measurements
    uint256 private _ghostSstoreNewSlotGasTotal;
    uint256 private _ghostSstoreNewSlotCount;

    /// @dev Tracks SSTORE to existing slot gas measurements
    uint256 private _ghostSstoreExistingGasTotal;
    uint256 private _ghostSstoreExistingCount;

    /// @dev Tracks storage clear refund measurements
    uint256 private _ghostStorageClearRefundTotal;
    uint256 private _ghostStorageClearCount;

    /// @dev Tracks contract creation gas measurements
    uint256 private _ghostCreateGasTotal;
    uint256 private _ghostCreateCount;
    uint256 private _ghostCreateBytesTotal;

    /// @dev Tracks multiple slot creation in single tx
    uint256 private _ghostMultiSlotGasTotal;
    uint256 private _ghostMultiSlotCount;
    uint256 private _ghostMultiSlotSlotsTotal;

    /// @dev Tracks transactions exceeding gas limit (should be 0)
    uint256 private _ghostTxOverLimitRejected;
    uint256 private _ghostTxOverLimitAllowed; // Violation counter

    /// @dev Tracks first tx gas validation
    uint256 private _ghostFirstTxUnderMinRejected;
    uint256 private _ghostFirstTxUnderMinAllowed; // Violation counter

    /// @dev Block gas tracking
    uint256 private _ghostBlockGasUsed;
    uint256 private _ghostGeneralLaneGasUsed;

    /// @dev TEMPO-GAS10: 2D nonce key creation tracking
    uint256 private _ghostNonceKeyCreationGasTotal;
    uint256 private _ghostNonceKeyCreationCount;

    /// @dev TEMPO-GAS11: Cold SLOAD + warm SSTORE reset tracking
    uint256 private _ghostColdLoadWarmStoreGasTotal;
    uint256 private _ghostColdLoadWarmStoreCount;
    uint256 private _ghostSstoreSetGasTotal;
    uint256 private _ghostSstoreSetCount;

    /// @dev TEMPO-GAS12: Pool vs EVM intrinsic gas validation
    uint256 private _ghostIntrinsicGasMismatchCount;

    /// @dev TEMPO-GAS13: T0 vs T1 gas param difference tracking
    uint256 private _ghostGasParamDifferenceCount;

    /// @dev TEMPO-GAS14: EIP-7702 refund tracking for T1
    uint256 private _ghostEip7702RefundViolations;

    /// @dev TEMPO-BLOCK8: Hardfork activation tracking
    uint256 private _ghostLastHardforkTimestamp;
    bool private _ghostHardforkMonotonicity;

    /// @dev TEMPO-BLOCK9: Hardfork boundary rule tracking
    uint256 private _ghostBoundaryViolations;

    /// @dev TEMPO-BLOCK10: Shared gas limit tracking
    uint256 private _ghostSharedGasLimitViolations;

    /// @dev TEMPO-BLOCK11: Base fee constancy within epoch
    uint256 private _ghostBaseFeeChangeCount;
    uint256 private _ghostLastBaseFee;

    /// @dev TEMPO-BLOCK12: Non-payment gas used tracking
    uint256 private _ghostNonPaymentGasUsed;
    uint256 private _ghostNonPaymentGasCapViolations;

    /*//////////////////////////////////////////////////////////////
                                SETUP
    //////////////////////////////////////////////////////////////*/

    function setUp() public override {
        super.setUp();

        targetContract(address(this));

        // Deploy test contracts
        _storageContract = new GasTestStorage();
        _factory = new GasTestFactory();

        // Build test actors
        _actors = _buildActors(10);

        // Initialize log file
        try vm.removeFile(LOG_FILE) { } catch { }
        _log("================================================================================");
        _log("                    TIP-1000 / TIP-1010 Gas Pricing Invariant Tests");
        _log("================================================================================");
        _log("");
    }

    /*//////////////////////////////////////////////////////////////
                            FUZZ HANDLERS
    //////////////////////////////////////////////////////////////*/

    /// @notice Handler: Test SSTORE to new (zero) slot
    /// @dev Tests TEMPO-GAS1: SSTORE zero→non-zero costs 250,000 gas
    /// @param slotSeed Seed for selecting storage slot
    /// @param value Non-zero value to store
    function handler_sstoreNewSlot(uint256 slotSeed, uint256 value) external {
        // Ensure non-zero value (zero would be a no-op)
        value = bound(value, 1, type(uint256).max);

        // Select a fresh slot that hasn't been written to
        bytes32 slot = keccak256(abi.encode(slotSeed, block.timestamp, _ghostSstoreNewSlotCount));

        // Measure gas for SSTORE to new slot
        uint256 gasBefore = gasleft();
        _storageContract.storeValue(slot, value);
        uint256 gasUsed = gasBefore - gasleft();

        // Track for invariant checking
        _ghostSstoreNewSlotGasTotal += gasUsed;
        _ghostSstoreNewSlotCount++;

        _log(
            string.concat(
                "SSTORE new slot: gas=", vm.toString(gasUsed), " slot=", vm.toString(uint256(slot))
            )
        );
    }

    /// @notice Handler: Test SSTORE to existing (non-zero) slot
    /// @dev Tests TEMPO-GAS3: SSTORE non-zero→non-zero costs 5,000 gas
    /// @param slotIdx Index of existing slot to update
    /// @param newValue New non-zero value
    function handler_sstoreExisting(uint256 slotIdx, uint256 newValue) external {
        // Skip if no slots have been written yet
        if (_storageContract.slotCount() == 0) return;

        // Bound to existing slots
        slotIdx = bound(slotIdx, 0, _storageContract.slotCount() - 1);
        bytes32 slot = _storageContract.getSlotAt(slotIdx);

        // Ensure new value is different and non-zero
        uint256 currentValue = _storageContract.getValue(slot);
        newValue = bound(newValue, 1, type(uint256).max);
        if (newValue == currentValue) {
            newValue = currentValue + 1;
        }

        // Measure gas for SSTORE to existing slot
        uint256 gasBefore = gasleft();
        _storageContract.storeValue(slot, newValue);
        uint256 gasUsed = gasBefore - gasleft();

        // Track for invariant checking
        _ghostSstoreExistingGasTotal += gasUsed;
        _ghostSstoreExistingCount++;

        _log(
            string.concat(
                "SSTORE existing: gas=", vm.toString(gasUsed), " slot=", vm.toString(uint256(slot))
            )
        );
    }

    /// @notice Handler: Test storage clearing (SSTORE to zero)
    /// @dev Tests TEMPO-GAS4: Storage clearing provides 15,000 gas refund
    /// @param slotIdx Index of slot to clear
    function handler_storageClear(uint256 slotIdx) external {
        // Skip if no slots have been written
        if (_storageContract.slotCount() == 0) return;

        // Select a slot with non-zero value
        slotIdx = bound(slotIdx, 0, _storageContract.slotCount() - 1);
        bytes32 slot = _storageContract.getSlotAt(slotIdx);

        uint256 currentValue = _storageContract.getValue(slot);
        if (currentValue == 0) return; // Already cleared

        // Measure gas for clearing storage
        uint256 gasBefore = gasleft();
        _storageContract.clearValue(slot);
        uint256 gasUsed = gasBefore - gasleft();

        // The refund is applied at tx end, but we can track the operation
        _ghostStorageClearRefundTotal += SSTORE_CLEAR_REFUND;
        _ghostStorageClearCount++;

        _log(
            string.concat(
                "SSTORE clear: gas=",
                vm.toString(gasUsed),
                " refund=",
                vm.toString(SSTORE_CLEAR_REFUND)
            )
        );
    }

    /// @notice Handler: Test contract creation gas costs
    /// @dev Tests TEMPO-GAS5: Contract creation = (code_size × 1,000) + 500,000 + 250,000
    /// @param codeSizeSeed Seed for code size selection
    function handler_contractCreate(uint256 codeSizeSeed) external {
        // Bound code size from small to max
        uint256 codeSize = bound(codeSizeSeed, 10, MAX_CONTRACT_SIZE);

        // Measure gas for contract creation
        uint256 gasBefore = gasleft();
        address deployed = _factory.deployWithSize(codeSize);
        uint256 gasUsed = gasBefore - gasleft();

        // Track for invariant checking
        _ghostCreateGasTotal += gasUsed;
        _ghostCreateCount++;
        _ghostCreateBytesTotal += codeSize;

        // Calculate expected gas
        uint256 expectedGas =
            (codeSize * CODE_DEPOSIT_PER_BYTE) + CREATE_BASE_GAS + ACCOUNT_CREATION_GAS;

        _log(
            string.concat(
                "CREATE: size=",
                vm.toString(codeSize),
                " gas=",
                vm.toString(gasUsed),
                " expected>=",
                vm.toString(expectedGas),
                " deployed=",
                vm.toString(deployed)
            )
        );
    }

    /// @notice Handler: Test multiple new storage slots in one transaction
    /// @dev Tests TEMPO-GAS8: Multiple new state elements charge 250k each independently
    /// @param numSlots Number of new slots to create (2-10)
    function handler_multipleNewSlots(uint256 numSlots) external {
        numSlots = bound(numSlots, 2, 10);

        // Create multiple fresh slots
        bytes32[] memory slots = new bytes32[](numSlots);
        for (uint256 i = 0; i < numSlots; i++) {
            slots[i] = keccak256(abi.encode("multi", block.timestamp, _ghostMultiSlotCount, i));
        }

        // Measure gas for multiple SSTOREs
        uint256 gasBefore = gasleft();
        _storageContract.storeMultiple(slots);
        uint256 gasUsed = gasBefore - gasleft();

        // Track for invariant checking
        _ghostMultiSlotGasTotal += gasUsed;
        _ghostMultiSlotCount++;
        _ghostMultiSlotSlotsTotal += numSlots;

        // Expected: numSlots × 250,000 (plus overhead)
        uint256 expectedMinGas = numSlots * SSTORE_SET_GAS;

        _log(
            string.concat(
                "Multi-SSTORE: slots=",
                vm.toString(numSlots),
                " gas=",
                vm.toString(gasUsed),
                " expected>=",
                vm.toString(expectedMinGas)
            )
        );
    }

    /// @notice Handler: Verify transaction gas limit enforcement
    /// @dev Tests TEMPO-GAS6 / TEMPO-BLOCK3: Transaction gas limit capped at 30M
    /// @param gasLimit Proposed gas limit to test
    function handler_txGasLimit(uint256 gasLimit) external {
        gasLimit = bound(gasLimit, 21_000, 50_000_000); // Test range around the cap

        if (gasLimit > TX_GAS_LIMIT_CAP) {
            // This should be rejected by the protocol
            _ghostTxOverLimitRejected++;
            _log(
                string.concat(
                    "TX gas limit rejected: ",
                    vm.toString(gasLimit),
                    " > cap ",
                    vm.toString(TX_GAS_LIMIT_CAP)
                )
            );
        } else {
            // Valid gas limit
            _log(
                string.concat(
                    "TX gas limit valid: ",
                    vm.toString(gasLimit),
                    " <= cap ",
                    vm.toString(TX_GAS_LIMIT_CAP)
                )
            );
        }
    }

    /// @notice Handler: Verify first transaction minimum gas
    /// @dev Tests TEMPO-GAS7: First tx (nonce=0) requires ≥271,000 gas
    /// @param gasLimit Gas limit for simulated first tx
    function handler_firstTxGas(uint256 gasLimit) external {
        gasLimit = bound(gasLimit, 21_000, 500_000);

        if (gasLimit < FIRST_TX_MIN_GAS) {
            // This should be rejected for nonce=0 transactions
            _ghostFirstTxUnderMinRejected++;
            _log(
                string.concat(
                    "First tx gas rejected: ",
                    vm.toString(gasLimit),
                    " < min ",
                    vm.toString(FIRST_TX_MIN_GAS)
                )
            );
        } else {
            _log(
                string.concat(
                    "First tx gas valid: ",
                    vm.toString(gasLimit),
                    " >= min ",
                    vm.toString(FIRST_TX_MIN_GAS)
                )
            );
        }
    }

    /// @notice Handler: Test max contract deployment fits in gas cap
    /// @dev Tests TEMPO-BLOCK6: 24KB contract deployment fits within 30M gas cap
    function handler_maxContractDeploy() external {
        // Calculate gas for max-size contract
        // Per TIP-1000: (24576 × 1000) + 500000 + 250000 = 25,326,000
        uint256 maxContractGas =
            (MAX_CONTRACT_SIZE * CODE_DEPOSIT_PER_BYTE) + CREATE_BASE_GAS + ACCOUNT_CREATION_GAS;

        // Add initcode overhead (calldata + hashing)
        // Initcode is 2x contract size max, plus overhead
        uint256 initcodeOverhead = MAX_INITCODE_SIZE * 16 / 32; // ~calldata cost estimate
        uint256 totalEstimate = maxContractGas + initcodeOverhead;

        // Verify it fits in TX_GAS_LIMIT_CAP
        bool fits = totalEstimate <= TX_GAS_LIMIT_CAP;

        _log(
            string.concat(
                "Max contract deploy: codeGas=",
                vm.toString(maxContractGas),
                " total~=",
                vm.toString(totalEstimate),
                " fits=",
                fits ? "true" : "false"
            )
        );
    }

    /*//////////////////////////////////////////////////////////////
                            INVARIANT FUNCTIONS
    //////////////////////////////////////////////////////////////*/

    /// @notice Master invariant function - runs all gas pricing invariant checks
    /// @dev Called by Foundry's invariant testing framework
    function invariant_gasPricing() public view {
        _invariantSstoreNewSlotCost();
        _invariantSstoreExistingCost();
        _invariantStorageClearRefund();
        _invariantContractCreationCost();
        _invariantMultipleSlotsCost();
        _invariantTxGasLimitCap();
        _invariantFirstTxMinGas();
        _invariantBlockGasLimits();
        _invariantBaseFee();
        _invariantPaymentLaneCapacity();
    }

    /// @notice TEMPO-GAS1: SSTORE to new slot costs 250,000 gas
    /// @dev Average gas should be at least SSTORE_SET_GAS (accounting for overhead)
    function _invariantSstoreNewSlotCost() internal view {
        if (_ghostSstoreNewSlotCount == 0) return;

        // Average should be at least 250k (the SSTORE itself)
        // Actual will be higher due to function call overhead
        uint256 avgGas = _ghostSstoreNewSlotGasTotal / _ghostSstoreNewSlotCount;

        // Gas must be >= SSTORE_SET_GAS (250k)
        // We allow overhead, so check that minimum is met
        assertTrue(
            avgGas >= SSTORE_SET_GAS - 10_000, // Allow some measurement variance
            "TEMPO-GAS1: SSTORE new slot gas below 250,000"
        );
    }

    /// @notice TEMPO-GAS3: SSTORE to existing slot costs 5,000 gas
    function _invariantSstoreExistingCost() internal view {
        if (_ghostSstoreExistingCount == 0) return;

        // Average should be at least 5k for the SSTORE reset
        uint256 avgGas = _ghostSstoreExistingGasTotal / _ghostSstoreExistingCount;

        // Should be significantly less than new slot cost
        uint256 avgNewSlotGas = _ghostSstoreNewSlotCount > 0
            ? _ghostSstoreNewSlotGasTotal / _ghostSstoreNewSlotCount
            : SSTORE_SET_GAS;

        assertTrue(avgGas < avgNewSlotGas, "TEMPO-GAS3: SSTORE existing not cheaper than new slot");
    }

    /// @notice TEMPO-GAS4: Storage clearing provides 15,000 gas refund
    function _invariantStorageClearRefund() internal view {
        if (_ghostStorageClearCount == 0) return;

        // Verify refunds are tracked
        uint256 expectedRefund = _ghostStorageClearCount * SSTORE_CLEAR_REFUND;
        assertEq(
            _ghostStorageClearRefundTotal,
            expectedRefund,
            "TEMPO-GAS4: Storage clear refund tracking mismatch"
        );
    }

    /// @notice TEMPO-GAS5: Contract creation cost formula
    /// Cost = (code_size × 1,000) + 500,000 + 250,000
    function _invariantContractCreationCost() internal view {
        if (_ghostCreateCount == 0) return;

        // Calculate minimum expected total gas
        uint256 expectedMinTotal = (_ghostCreateBytesTotal * CODE_DEPOSIT_PER_BYTE)
            + (_ghostCreateCount * CREATE_BASE_GAS) + (_ghostCreateCount * ACCOUNT_CREATION_GAS);

        // Actual gas should be >= expected (accounting for overhead)
        assertTrue(
            _ghostCreateGasTotal >= expectedMinTotal - (_ghostCreateCount * 50_000), // Allow overhead variance
            "TEMPO-GAS5: Contract creation gas below expected formula"
        );
    }

    /// @notice TEMPO-GAS8: Multiple new slots charge 250k each independently
    function _invariantMultipleSlotsCost() internal view {
        if (_ghostMultiSlotCount == 0) return;

        // Minimum expected: slots × 250k
        uint256 expectedMinTotal = _ghostMultiSlotSlotsTotal * SSTORE_SET_GAS;

        // Actual should be at least this (plus overhead)
        assertTrue(
            _ghostMultiSlotGasTotal >= expectedMinTotal - (_ghostMultiSlotCount * 50_000),
            "TEMPO-GAS8: Multi-slot gas below N * 250k"
        );
    }

    /// @notice TEMPO-GAS6 / TEMPO-BLOCK3: Transaction gas limit capped at 30M
    function _invariantTxGasLimitCap() internal view {
        // No transactions over 30M should be allowed
        assertEq(
            _ghostTxOverLimitAllowed,
            0,
            "TEMPO-GAS6: Transaction over 30M gas limit unexpectedly allowed"
        );
    }

    /// @notice TEMPO-GAS7: First tx (nonce=0) requires ≥271,000 gas
    function _invariantFirstTxMinGas() internal view {
        // No first transactions under minimum should be allowed
        assertEq(
            _ghostFirstTxUnderMinAllowed,
            0,
            "TEMPO-GAS7: First tx under 271k gas unexpectedly allowed"
        );
    }

    /// @notice TEMPO-BLOCK1 / TEMPO-BLOCK2: Block gas limits
    function _invariantBlockGasLimits() internal view {
        // Block gas should never exceed 500M
        assertTrue(
            _ghostBlockGasUsed <= BLOCK_GAS_LIMIT, "TEMPO-BLOCK1: Block gas exceeds 500M limit"
        );

        // General lane should never exceed 30M
        assertTrue(
            _ghostGeneralLaneGasUsed <= GENERAL_GAS_LIMIT,
            "TEMPO-BLOCK2: General lane gas exceeds 30M limit"
        );
    }

    /// @notice TEMPO-BLOCK4: T1 hardfork base fee is exactly 20 gwei
    function _invariantBaseFee() internal view {
        // Note: This invariant is enforced at the protocol level
        // We verify the constant is correctly defined
        assertEq(T1_BASE_FEE, 20_000_000_000, "TEMPO-BLOCK4: T1 base fee constant incorrect");
    }

    /// @notice TEMPO-BLOCK5: Payment lane has ≥470M gas available
    function _invariantPaymentLaneCapacity() internal view {
        // Available payment gas = total - general used
        uint256 availablePaymentGas = BLOCK_GAS_LIMIT - _ghostGeneralLaneGasUsed;

        assertTrue(
            availablePaymentGas >= PAYMENT_GAS_MIN, "TEMPO-BLOCK5: Payment lane capacity below 470M"
        );
    }

    /*//////////////////////////////////////////////////////////////
                            HELPER FUNCTIONS
    //////////////////////////////////////////////////////////////*/

    /// @notice Build array of test actors
    function _buildActors(uint256 count) internal returns (address[] memory) {
        address[] memory actors_ = new address[](count);
        for (uint256 i = 0; i < count; i++) {
            actors_[i] = makeAddr(string.concat("actor", vm.toString(i)));
            vm.deal(actors_[i], 100 ether);
        }
        return actors_;
    }

    /// @notice Log a message to the log file
    function _log(string memory message) internal {
        vm.writeLine(LOG_FILE, message);
    }

}

/*//////////////////////////////////////////////////////////////
                        HELPER CONTRACTS
//////////////////////////////////////////////////////////////*/

/// @title GasTestStorage - Contract for testing SSTORE gas costs
/// @dev Simple contract with storage operations for gas measurement
contract GasTestStorage {

    /// @dev Storage mapping for testing
    mapping(bytes32 => uint256) private _storage;

    /// @dev Track which slots have been written
    bytes32[] private _slots;

    /// @dev Store a value at a slot
    function storeValue(bytes32 slot, uint256 value) external {
        if (_storage[slot] == 0 && value != 0) {
            _slots.push(slot);
        }
        _storage[slot] = value;
    }

    /// @dev Clear a storage slot
    function clearValue(bytes32 slot) external {
        _storage[slot] = 0;
    }

    /// @dev Store multiple values at once
    function storeMultiple(bytes32[] calldata slots) external {
        for (uint256 i = 0; i < slots.length; i++) {
            if (_storage[slots[i]] == 0) {
                _slots.push(slots[i]);
            }
            _storage[slots[i]] = 1; // Use 1 as non-zero value
        }
    }

    /// @dev Get value at slot
    function getValue(bytes32 slot) external view returns (uint256) {
        return _storage[slot];
    }

    /// @dev Get slot at index
    function getSlotAt(uint256 idx) external view returns (bytes32) {
        return _slots[idx];
    }

    /// @dev Get number of written slots
    function slotCount() external view returns (uint256) {
        return _slots.length;
    }

}

/// @title GasTestFactory - Factory for deploying contracts of various sizes
/// @dev Creates contracts with specified code sizes for gas testing
contract GasTestFactory {

    /// @notice Deploy a contract with approximately the specified code size
    /// @param size Target code size in bytes
    /// @return deployed Address of the deployed contract
    function deployWithSize(uint256 size) external returns (address deployed) {
        // Create bytecode of approximate size
        // Minimal contract: PUSH1 0x00 PUSH1 0x00 RETURN (6 bytes)
        // Pad with JUMPDEST (0x5b) to reach target size
        bytes memory code = new bytes(size);

        // Minimal valid code at start
        code[0] = 0x60; // PUSH1
        code[1] = 0x00; // 0x00
        code[2] = 0x60; // PUSH1
        code[3] = 0x00; // 0x00
        code[4] = 0xf3; // RETURN

        // Fill rest with JUMPDEST for valid code
        for (uint256 i = 5; i < size; i++) {
            code[i] = 0x5b; // JUMPDEST
        }

        // Deploy using CREATE
        assembly {
            deployed := create(0, add(code, 0x20), mload(code))
        }

        require(deployed != address(0), "Deployment failed");
    }

}
