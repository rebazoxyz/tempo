// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IBridge} from "./interfaces/IBridge.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/// @title WrappedToken
/// @notice ERC20 token representing a wrapped asset from another chain
contract WrappedToken is ERC20, Ownable {
    uint8 private immutable _decimals;

    constructor(
        string memory name_,
        string memory symbol_,
        uint8 decimals_,
        address bridge_
    ) ERC20(name_, symbol_) Ownable(bridge_) {
        _decimals = decimals_;
    }

    function decimals() public view override returns (uint8) {
        return _decimals;
    }

    function mint(address to, uint256 amount) external onlyOwner {
        _mint(to, amount);
    }

    function burn(address from, uint256 amount) external onlyOwner {
        _burn(from, amount);
    }
}

/// @title TokenBridge
/// @notice ICS20-inspired token transfer application for the bridge
/// @dev Handles lock/mint and burn/unlock token transfer patterns
contract TokenBridge {
    using SafeERC20 for IERC20;

    /// @notice Token transfer packet data
    struct TransferPacket {
        /// @dev Token denomination (address on source chain or prefixed denom)
        string denom;
        /// @dev Amount to transfer
        uint256 amount;
        /// @dev Sender address on source chain
        address sender;
        /// @dev Recipient address on destination chain
        address receiver;
        /// @dev Optional memo
        string memo;
    }

    /// @notice Denomination trace for wrapped tokens
    struct DenomTrace {
        /// @dev Full path of the denomination (e.g., "transfer/channel-0/uatom")
        string path;
        /// @dev Base denomination
        string baseDenom;
    }

    /// @notice The bridge router contract
    IBridge public immutable bridge;

    /// @dev Escrow balance for locked tokens: token => amount
    mapping(address => uint256) private _escrowBalances;

    /// @dev Wrapped token contracts: denomHash => wrappedToken
    mapping(bytes32 => WrappedToken) private _wrappedTokens;

    /// @dev Denomination traces: denomHash => trace
    mapping(bytes32 => DenomTrace) private _denomTraces;

    /// @dev Check if a denom is native to this chain: denomHash => isNative
    mapping(bytes32 => bool) private _nativeDenoms;

    /// @dev Reverse mapping: wrapped token address => denomHash
    mapping(address => bytes32) private _tokenToDenom;

    /// @dev Port ID for this application
    string public constant PORT_ID = "transfer";

    /// @notice Emitted when tokens are transferred out
    event TransferSent(
        uint64 indexed sequence,
        address indexed sender,
        address indexed receiver,
        string denom,
        uint256 amount,
        string destClient
    );

    /// @notice Emitted when tokens are received
    event TransferReceived(
        uint64 indexed sequence,
        address indexed receiver,
        string denom,
        uint256 amount,
        string sourceClient
    );

    /// @notice Emitted when a transfer is acknowledged
    event TransferAcknowledged(uint64 indexed sequence, bool success);

    /// @notice Emitted when a transfer times out and tokens are refunded
    event TransferRefunded(
        uint64 indexed sequence,
        address indexed sender,
        string denom,
        uint256 amount
    );

    /// @notice Emitted when a new wrapped token is created
    event WrappedTokenCreated(
        bytes32 indexed denomHash,
        address indexed tokenAddress,
        string denom,
        string name,
        string symbol
    );

    /// @notice Error when amount is zero
    error ZeroAmount();

    /// @notice Error when receiver is zero address
    error InvalidReceiver();

    /// @notice Error when token transfer fails
    error TransferFailed();

    /// @notice Error when caller is not the bridge
    error OnlyBridge();

    /// @notice Error when acknowledgement indicates failure
    error TransferRejected(string reason);

    /// @notice Error when denom is not found
    error DenomNotFound(string denom);

    modifier onlyBridge() {
        if (msg.sender != address(bridge)) revert OnlyBridge();
        _;
    }

    constructor(IBridge _bridge) {
        bridge = _bridge;
    }

    /// @notice Transfer tokens to another chain
    /// @param destClient Destination client ID
    /// @param token Token address to transfer (address(0) for native ETH)
    /// @param amount Amount to transfer
    /// @param receiver Recipient address on destination chain
    /// @param timeout Timeout timestamp in nanoseconds
    /// @param memo Optional memo
    /// @return sequence The packet sequence number
    function transfer(
        string calldata destClient,
        address token,
        uint256 amount,
        address receiver,
        uint64 timeout,
        string calldata memo
    ) external payable returns (uint64 sequence) {
        if (amount == 0) revert ZeroAmount();
        if (receiver == address(0)) revert InvalidReceiver();

        string memory denom;
        bytes32 denomHash = _tokenToDenom[token];

        if (denomHash != bytes32(0) && address(_wrappedTokens[denomHash]) == token) {
            // This is a wrapped token - burn it
            DenomTrace storage trace = _denomTraces[denomHash];
            denom = string(abi.encodePacked(trace.path, "/", trace.baseDenom));

            WrappedToken(token).burn(msg.sender, amount);
        } else {
            // This is a native token - lock it
            denom = _addressToString(token);
            denomHash = keccak256(bytes(denom));
            _nativeDenoms[denomHash] = true;

            IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
            _escrowBalances[token] += amount;
        }

        // Create transfer packet
        TransferPacket memory transferData = TransferPacket({
            denom: denom,
            amount: amount,
            sender: msg.sender,
            receiver: receiver,
            memo: memo
        });

        bytes memory payload = abi.encode(transferData);

        sequence = bridge.sendPacket(destClient, payload, timeout);

        emit TransferSent(sequence, msg.sender, receiver, denom, amount, destClient);
    }

    /// @notice Handle incoming packet (called by bridge)
    /// @param packet The received packet
    function onRecvPacket(
        IBridge.Packet calldata packet
    ) external onlyBridge returns (bytes memory ack) {
        TransferPacket memory transferData = abi.decode(packet.payload, (TransferPacket));

        bytes32 denomHash = keccak256(bytes(transferData.denom));

        // Check if this is a token returning to its native chain
        if (_nativeDenoms[denomHash]) {
            // Unlock escrowed tokens
            address token = _stringToAddress(transferData.denom);
            _escrowBalances[token] -= transferData.amount;
            IERC20(token).safeTransfer(transferData.receiver, transferData.amount);
        } else {
            // Mint wrapped tokens
            string memory prefixedDenom = string(
                abi.encodePacked(PORT_ID, "/", packet.sourceClient, "/", transferData.denom)
            );
            bytes32 prefixedDenomHash = keccak256(bytes(prefixedDenom));

            WrappedToken wrappedToken = _wrappedTokens[prefixedDenomHash];

            if (address(wrappedToken) == address(0)) {
                // Create new wrapped token
                wrappedToken = _createWrappedToken(
                    prefixedDenomHash,
                    prefixedDenom,
                    transferData.denom
                );
            }

            wrappedToken.mint(transferData.receiver, transferData.amount);
        }

        emit TransferReceived(
            packet.sequence,
            transferData.receiver,
            transferData.denom,
            transferData.amount,
            packet.sourceClient
        );

        // Return success acknowledgement
        return abi.encode(true, "");
    }

    /// @notice Handle acknowledgement (called by bridge)
    /// @param packet The original packet
    /// @param ack The acknowledgement data
    function onAcknowledgePacket(
        IBridge.Packet calldata packet,
        bytes calldata ack
    ) external onlyBridge {
        (bool success, string memory errorMsg) = abi.decode(ack, (bool, string));

        emit TransferAcknowledged(packet.sequence, success);

        if (!success) {
            // Refund on failure
            _refundTokens(packet);
            revert TransferRejected(errorMsg);
        }
    }

    /// @notice Handle timeout (called by bridge)
    /// @param packet The timed out packet
    function onTimeoutPacket(IBridge.Packet calldata packet) external onlyBridge {
        _refundTokens(packet);
    }

    /// @notice Get the wrapped token address for a denomination
    /// @param denom The denomination string
    /// @return token The wrapped token address
    function getWrappedToken(string calldata denom) external view returns (address) {
        bytes32 denomHash = keccak256(bytes(denom));
        return address(_wrappedTokens[denomHash]);
    }

    /// @notice Get the denomination trace for a denom hash
    /// @param denomHash The hash of the denomination
    /// @return trace The denomination trace
    function getDenomTrace(bytes32 denomHash) external view returns (DenomTrace memory) {
        return _denomTraces[denomHash];
    }

    /// @notice Check if a denomination is native to this chain
    /// @param denom The denomination string
    /// @return isNative True if the denom is native
    function isNativeDenom(string calldata denom) external view returns (bool) {
        bytes32 denomHash = keccak256(bytes(denom));
        return _nativeDenoms[denomHash];
    }

    /// @notice Get escrowed balance for a token
    /// @param token The token address
    /// @return balance The escrowed amount
    function getEscrowBalance(address token) external view returns (uint256) {
        return _escrowBalances[token];
    }

    /// @dev Create a new wrapped token
    function _createWrappedToken(
        bytes32 denomHash,
        string memory fullDenom,
        string memory baseDenom
    ) private returns (WrappedToken) {
        // Generate name and symbol from base denom
        string memory name = string(abi.encodePacked("Wrapped ", baseDenom));
        string memory symbol = string(abi.encodePacked("w", _truncate(baseDenom, 10)));

        WrappedToken wrappedToken = new WrappedToken(name, symbol, 18, address(this));

        _wrappedTokens[denomHash] = wrappedToken;
        _tokenToDenom[address(wrappedToken)] = denomHash;
        _denomTraces[denomHash] = DenomTrace({path: fullDenom, baseDenom: baseDenom});

        emit WrappedTokenCreated(denomHash, address(wrappedToken), fullDenom, name, symbol);

        return wrappedToken;
    }

    /// @dev Refund tokens on failure or timeout
    function _refundTokens(IBridge.Packet calldata packet) private {
        TransferPacket memory transferData = abi.decode(packet.payload, (TransferPacket));

        bytes32 denomHash = keccak256(bytes(transferData.denom));

        if (_nativeDenoms[denomHash]) {
            // Return escrowed tokens
            address token = _stringToAddress(transferData.denom);
            _escrowBalances[token] -= transferData.amount;
            IERC20(token).safeTransfer(transferData.sender, transferData.amount);
        } else {
            // Re-mint wrapped tokens
            WrappedToken wrappedToken = _wrappedTokens[denomHash];
            if (address(wrappedToken) != address(0)) {
                wrappedToken.mint(transferData.sender, transferData.amount);
            }
        }

        emit TransferRefunded(
            packet.sequence,
            transferData.sender,
            transferData.denom,
            transferData.amount
        );
    }

    /// @dev Convert address to string (for denomination)
    function _addressToString(address addr) private pure returns (string memory) {
        bytes memory alphabet = "0123456789abcdef";
        bytes memory data = abi.encodePacked(addr);
        bytes memory str = new bytes(2 + data.length * 2);
        str[0] = "0";
        str[1] = "x";
        for (uint256 i = 0; i < data.length; i++) {
            str[2 + i * 2] = alphabet[uint8(data[i] >> 4)];
            str[3 + i * 2] = alphabet[uint8(data[i] & 0x0f)];
        }
        return string(str);
    }

    /// @dev Convert string to address
    function _stringToAddress(string memory str) private pure returns (address) {
        bytes memory strBytes = bytes(str);
        require(strBytes.length == 42, "Invalid address string length");
        require(strBytes[0] == "0" && strBytes[1] == "x", "Invalid address prefix");

        uint160 addr = 0;
        for (uint256 i = 2; i < 42; i++) {
            uint8 b = uint8(strBytes[i]);
            uint8 digit;
            if (b >= 48 && b <= 57) {
                digit = b - 48; // '0'-'9'
            } else if (b >= 97 && b <= 102) {
                digit = b - 87; // 'a'-'f'
            } else if (b >= 65 && b <= 70) {
                digit = b - 55; // 'A'-'F'
            } else {
                revert("Invalid hex character");
            }
            addr = addr * 16 + digit;
        }
        return address(addr);
    }

    /// @dev Truncate a string to max length
    function _truncate(string memory str, uint256 maxLen) private pure returns (string memory) {
        bytes memory strBytes = bytes(str);
        if (strBytes.length <= maxLen) {
            return str;
        }
        bytes memory truncated = new bytes(maxLen);
        for (uint256 i = 0; i < maxLen; i++) {
            truncated[i] = strBytes[i];
        }
        return string(truncated);
    }
}
