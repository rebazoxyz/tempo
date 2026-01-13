// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {Vm} from "forge-std/Vm.sol";

import {BaseTest} from "./BaseTest.t.sol";
import {TIP20} from "../src/TIP20.sol";
import {INonce} from "../src/interfaces/INonce.sol";
import {ITIP20} from "../src/interfaces/ITIP20.sol";

import {VmRlp, VmExecuteTransaction} from "tempo-std/StdVm.sol";
import {TempoTransaction, TempoCall, TempoTransactionLib} from "tempo-std/tx/TempoTransactionLib.sol";

/// @title Tempo Transaction Invariant Tests
/// @notice Proper Foundry invariant tests for 2D nonce behavior using TempoTransaction
/// @dev Handler functions are in the test contract itself since vm.sign requires test context
contract TempoTransactionInvariantTest is BaseTest {
    using TempoTransactionLib for TempoTransaction;

    VmRlp internal vmRlp = VmRlp(address(vm));
    VmExecuteTransaction internal vmExec = VmExecuteTransaction(address(vm));

    TIP20 feeToken;
    address validator;

    address[] public actors;
    uint256[] actorKeys;

    // Ghost variables for invariant checking (tracking nonce for key 0)
    mapping(address => uint256) public ghost_expectedNonce;
    uint256 public ghost_totalTxExecuted;
    uint256 public ghost_totalTxReverted;

    function setUp() public override {
        super.setUp();

        feeToken = TIP20(
            factory.createToken("Fee Token", "FEE", "USD", pathUSD, admin, bytes32("feetoken"))
        );

        validator = makeAddr("validator");

        for (uint256 i = 1; i <= 5; i++) {
            (address actor, uint256 pk) = makeAddrAndKey(
                string(abi.encodePacked("actor", vm.toString(i)))
            );
            actors.push(actor);
            actorKeys.push(pk);
            ghost_expectedNonce[actor] = 0;
        }

        vm.startPrank(admin);
        feeToken.grantRole(_ISSUER_ROLE, admin);
        for (uint256 i = 0; i < actors.length; i++) {
            feeToken.mint(actors[i], 10_000_000e6);
        }
        vm.stopPrank();

        // Target this contract for handler functions
        targetContract(address(this));

        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = this.handler_transfer.selector;
        selectors[1] = this.handler_sequentialTransfers.selector;
        targetSelector(FuzzSelector({addr: address(this), selectors: selectors}));
    }

    /*//////////////////////////////////////////////////////////////
                        TRANSACTION BUILDING
    //////////////////////////////////////////////////////////////*/

    function _buildAndSignTempoTx(
        uint256 actorIndex,
        address to,
        bytes memory data,
        uint64 gasLimit,
        uint256 maxFeePerGas,
        uint64 txNonce
    ) internal view returns (bytes memory) {
        TempoCall[] memory calls = new TempoCall[](1);
        calls[0] = TempoCall({to: to, value: 0, data: data});

        TempoTransaction memory tx_ = TempoTransactionLib
            .create()
            .withChainId(uint64(block.chainid))
            .withMaxFeePerGas(maxFeePerGas)
            .withGasLimit(gasLimit)
            .withCalls(calls)
            .withNonceKey(0)
            .withNonce(txNonce);

        bytes memory unsignedTx = tx_.encode(vmRlp);
        bytes32 txHash = keccak256(unsignedTx);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(actorKeys[actorIndex], txHash);

        return tx_.encodeWithSignature(vmRlp, v, r, s);
    }

    /*//////////////////////////////////////////////////////////////
                            HANDLERS
    //////////////////////////////////////////////////////////////*/

    /// @notice Handler: Execute a transfer from a random actor
    /// @param actorSeed Seed to select sender
    /// @param recipientSeed Seed to select recipient  
    /// @param amount Amount to transfer (will be bounded)
    function handler_transfer(
        uint256 actorSeed,
        uint256 recipientSeed,
        uint256 amount
    ) external {
        uint256 senderIdx = actorSeed % actors.length;
        uint256 recipientIdx = recipientSeed % actors.length;
        if (senderIdx == recipientIdx) {
            recipientIdx = (recipientIdx + 1) % actors.length;
        }

        address sender = actors[senderIdx];
        address recipient = actors[recipientIdx];

        amount = bound(amount, 1e6, 100e6);

        uint256 balance = feeToken.balanceOf(sender);
        if (balance < amount) {
            return;
        }

        uint64 currentNonce = uint64(ghost_expectedNonce[sender]);

        bytes memory signedTx = _buildAndSignTempoTx(
            senderIdx,
            address(feeToken),
            abi.encodeCall(ITIP20.transfer, (recipient, amount)),
            100_000,
            100,
            currentNonce
        );

        vm.coinbase(validator);

        try vmExec.executeTransaction(signedTx) {
            ghost_expectedNonce[sender]++;
            ghost_totalTxExecuted++;
        } catch {
            ghost_totalTxReverted++;
        }
    }

    /// @notice Handler: Execute multiple transfers in sequence from same actor
    /// @param actorSeed Seed to select sender
    /// @param count Number of transfers (bounded 1-5)
    function handler_sequentialTransfers(
        uint256 actorSeed,
        uint256 count
    ) external {
        count = bound(count, 1, 5);
        uint256 senderIdx = actorSeed % actors.length;
        uint256 recipientIdx = (senderIdx + 1) % actors.length;

        address sender = actors[senderIdx];
        address recipient = actors[recipientIdx];

        uint256 amountPerTx = 10e6;
        uint256 balance = feeToken.balanceOf(sender);

        if (balance < amountPerTx * count) {
            return;
        }

        for (uint256 i = 0; i < count; i++) {
            uint64 currentNonce = uint64(ghost_expectedNonce[sender]);

            bytes memory signedTx = _buildAndSignTempoTx(
                senderIdx,
                address(feeToken),
                abi.encodeCall(ITIP20.transfer, (recipient, amountPerTx)),
                100_000,
                100,
                currentNonce
            );

            vm.coinbase(validator);

            try vmExec.executeTransaction(signedTx) {
                ghost_expectedNonce[sender]++;
                ghost_totalTxExecuted++;
            } catch {
                ghost_totalTxReverted++;
                break;
            }
        }
    }

    /*//////////////////////////////////////////////////////////////
                            INVARIANTS
    //////////////////////////////////////////////////////////////*/

    /// @notice INVARIANT: Actual 2D nonce (key 0) always equals expected nonce (ghost variable)
    /// @dev After any sequence of handler calls, each actor's on-chain nonce
    ///      must equal the number of successful txs we tracked
    function invariant_nonceMatchesExpected() public view {
        for (uint256 i = 0; i < actors.length; i++) {
            address actor = actors[i];
            uint256 actualNonce = nonce.getNonce(actor, 0);
            uint256 expectedNonce = ghost_expectedNonce[actor];

            assertEq(
                actualNonce,
                expectedNonce,
                string(abi.encodePacked(
                    "Nonce mismatch for actor ",
                    vm.toString(i)
                ))
            );
        }
    }

    /// @notice INVARIANT: Nonces are monotonically increasing (never decrease)
    function invariant_nonceMonotonic() public view {
        for (uint256 i = 0; i < actors.length; i++) {
            address actor = actors[i];
            uint256 actorNonce = nonce.getNonce(actor, 0);
            assertGe(actorNonce, 0, "Nonce should never be negative");
        }
    }

    /// @notice INVARIANT: Total executed txs equals sum of all actor nonces
    function invariant_txCountingConsistent() public view {
        uint256 sumOfNonces = 0;
        for (uint256 i = 0; i < actors.length; i++) {
            sumOfNonces += ghost_expectedNonce[actors[i]];
        }
        assertEq(
            sumOfNonces,
            ghost_totalTxExecuted,
            "Sum of nonces should equal total executed txs"
        );
    }

}
