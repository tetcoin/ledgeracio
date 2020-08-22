// Copyright 2020 Parity Technologies (UK) Ltd.
// This file is part of ledgeracio.
//
// ledgeracio is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// ledgeracio is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with ledgeracio.  If not, see <http://www.gnu.org/licenses/>.

//! The main binary of Ledgeracio

#![deny(clippy::all)]
#![allow(clippy::non_ascii_literal)]
#![forbid(unsafe_code)]
#[cfg(feature = "allowlist")]
mod approved_validators;
mod common;
mod derivation;
mod hardstore;
mod keyparse;
mod mock;
mod nominator;
mod parser;
mod payouts;
mod validator;

use clap::arg_enum;
use codec::Encode;
use derivation::{AccountType, LedgeracioPath};
use futures::future::TryFutureExt;
use hardstore::HardStore;

#[cfg(not(unix))]
compile_error!("Only *nix-like platforms are supported");

use common::AddressSource;
use sp_core::crypto::AccountId32 as AccountId;
use std::{convert::{TryFrom, TryInto},
          fmt::Debug,
          future::Future,
          pin::Pin};
use structopt::StructOpt;
use substrate_subxt::{sp_core,
                      sp_core::crypto::{Ss58AddressFormat, Ss58Codec},
                      staking::RewardDestination,
                      Client, ClientBuilder, Signer};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// The version of keys supported
const KEY_VERSION: u8 = 1;

/// The magic number at the beginning of a secret key
const KEY_MAGIC: &[u8] = &*b"Ledgeracio Secret Key";

/// Output format
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum OutputFormat {
    /// Human-readable formatted text
    Text,
    /// Machine-parsable JSON output
    JSON,
    /// Spreadsheet-importable CSV output
    CSV,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "JSON" => Ok(Self::JSON),
            "CSV" => Ok(Self::CSV),
            "Text" => Ok(Self::Text),
            _ => Err(format!("invalid output format {:?}", s)),
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "Ledgeracio", about = "Ledger CLI for staking")]
struct Ledgeracio {
    /// Dry run.  Do not execute the operation.
    #[structopt(short = "n", long)]
    dry_run: bool,
    /// RPC host
    #[structopt(short, long)]
    host: Option<String>,
    /// Network
    #[structopt(long)]
    network: Network,
    /// Subcommand
    #[structopt(subcommand)]
    cmd: Command,
}

arg_enum! {
    #[derive(Copy, Clone, Debug)]
    enum Network {
        // The Kusama (canary) network
        Kusama,
        // The Polkadot (live) network
        Polkadot,
    }
}

async fn display_path(
    account_type: AccountType,
    keystore: &HardStore,
    network: Ss58AddressFormat,
    index: u32,
) -> Result<(), Error> {
    if index == 0 {
        return Err("Index must not be zero".to_owned().into())
    }
    let path = LedgeracioPath::new(network, account_type, index)?;
    let signer: hardstore::HardSigner = keystore.signer(path).await?;
    let account_id: &AccountId = signer.account_id();
    println!("{}", account_id.to_ss58check_with_version(network));
    Ok(())
}

#[derive(StructOpt, Debug)]
enum Command {
    /// Nominator operations
    Nominator(nominator::Nominator),
    /// Validator operations
    Validator(validator::Validator),
    /// Allowlist operations
    Allowlist(approved_validators::ACL),
    /// Pretty-print the chain metadata
    Metadata,
    /// Display the chain properties
    Properties,
}

type Runtime = substrate_subxt::KusamaRuntime;

fn parse_reward_destination(arg: &str) -> Result<RewardDestination, &'static str> {
    Ok(match &*arg.to_ascii_lowercase() {
        "staked" => RewardDestination::Staked,
        "stash" => RewardDestination::Stash,
        "controller" => RewardDestination::Controller,
        _ => return Err("bad reward destination ― must be Staked, Stash, or Controller"),
    })
}

/// Parse an SS58 address
pub(crate) fn parse_address<T: Ss58Codec>(arg: &str) -> Result<(T, u8), String> {
    Ss58Codec::from_string_with_version(arg)
        .map_err(|e| format!("{:?}", e))
        .map(|(x, y)| (x, y.into()))
}

#[async_std::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let Ledgeracio {
        dry_run,
        host,
        network,
        cmd,
    } = Ledgeracio::from_args();
    let address_format = match network {
        Network::Kusama => Ss58AddressFormat::KusamaAccount,
        Network::Polkadot => Ss58AddressFormat::PolkadotAccount,
    };
    let host = host.unwrap_or_else(|| {
        match network {
            Network::Kusama => "wss://kusama-rpc.polkadot.io",
            Network::Polkadot => "wss://rpc.polkadot.io",
        }
        .to_owned()
    });

    let client = ClientBuilder::<Runtime>::new()
        .set_url(host)
        .build()
        .map_err(From::from);
    let client: Pin<Box<dyn Future<Output = Result<Client<Runtime>, _>>>> = Box::pin(client);
    let keystore = || hardstore::HardStore::new(network);
    if dry_run {
        return Ok(())
    }
    if let Some(hash) = match cmd {
        Command::Nominator(s) => nominator::main(s, client, address_format, keystore).await?,
        Command::Validator(v) => validator::main(v, client, address_format, keystore).await?,
        Command::Allowlist(l) => approved_validators::main(l, keystore, address_format).await?,
        Command::Metadata => {
            println!("{:#?}", client.await?.metadata());
            None
        }
        Command::Properties => {
            println!("{:#?}", client.await?.properties());
            None
        }
    } {
        println!("Transaction hash: {:?}", hash);
    }
    Ok(())
}

fn validate_network(
    address: &str,
    provided_network: u8,
    network: Ss58AddressFormat,
) -> Result<(), Error> {
    if network == provided_network.try_into().unwrap() {
        Ok(())
    } else {
        Err(format!(
            "Network mismatch: address {} is for network {}, but you asked to use network {}",
            address,
            String::from(Ss58AddressFormat::try_from(provided_network).unwrap()),
            String::from(network),
        )
        .into())
    }
}
