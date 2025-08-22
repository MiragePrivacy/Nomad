use std::fmt::Display;

use alloy::signers::local::PrivateKeySigner;
use clap::Subcommand;
use color_eyre::Result;
use paste::paste;

use crate::config::Config;

macro_rules! impl_command {
    {
        $( #[doc = $doc:expr] $( #[ display = $func:expr ] )? mod $mod:tt; )*
        pub struct $id:ident;
    } => {
        macro_rules! impl_display {
            ($id1:tt) => { stringify!($id1) };
            ($id1:tt, $args:expr, $func1:tt ) => { $args.$func1().as_str() };
        }

        paste! {
            $( mod $mod; )*

            #[derive(Subcommand)]
            pub enum $id {
                $( #[doc = $doc] [< $mod:camel >](Box<$mod::[< $mod:camel Args >]>) ),*
            }

            impl $id {
                pub async fn execute(self, config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
                    match self {
                        $( Self::[< $mod:camel >](args) => args.execute(config, signers).await, )*
                    }
                }
            }

            impl Display for $id {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    match self {
                        $( Self::[< $mod:camel >](_args) => f.write_str(
                            impl_display!($mod $(, _args, $func)?)
                        ), )*
                    }
                }
            }
        }
    };
}

impl_command! {
    /// Run the node. If no keys are included, runs in read-only mode.
    mod run;
    /// Call RPC methods on a local or remote node.
    #[display = to_string]
    mod rpc;
    /// Use the faucet functionality on the given token contract. Requires keys.
    mod faucet;

    pub struct Command;
}
