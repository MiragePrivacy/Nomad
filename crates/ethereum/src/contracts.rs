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
        function getAmountsIn(uint amountOut, address[] calldata path)
            external view returns (uint[] memory amounts);
        function WETH() external pure returns (address);
        function factory() external pure returns (address);
    }
}
