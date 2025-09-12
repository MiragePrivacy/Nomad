alloy::sol! {
    #[sol(rpc)]
    contract IERC20 {
        event Transfer(address indexed from, address indexed to, uint256 value);

        function balanceOf(address) public view returns (uint256);
        function mint() external;
        function transfer(address to, uint256 value) external returns (bool);
        function approve(address spender, uint256 value) external returns (bool);
        function decimals() public view returns (uint8);
    }

    #[sol(rpc)]
    contract IUniswapV2Router02 {
        function swapExactTokensForETH(
            uint amountIn,
            uint amountOutMin,
            address[] calldata path,
            address to,
            uint deadline
        ) external returns (uint[] memory amounts);
        function getAmountsOut(uint amountIn, address[] calldata path)
            external view returns (uint[] memory amounts);
        function WETH() external pure returns (address);
        function factory() external pure returns (address);
    }

    #[sol(rpc)]
    contract Escrow {
        #[derive(serde::Deserialize, serde::Serialize)]
        struct ReceiptProof {
            /// RLP-encoded block header
            bytes header;
            /// RLP-encoded target receipt
            bytes receipt;
            /// Serialized MPT proof nodes
            bytes proof;
            /// RLP-encoded receipt index
            bytes path;
            /// Index of target log in receipt
            uint256 log;
        }

        function bond(uint256 _bondAmount) public;
        function collect(ReceiptProof calldata proof, uint256 targetBlockNumber) public;
        function is_bonded() public view returns (bool);
    }
}

impl std::fmt::Display for Escrow::ReceiptProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&serde_json::to_string_pretty(self).unwrap())
    }
}
