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

//! Stash commands

use super::{parse_address, AccountId, AccountType, Error, LedgeracioPath, StructOpt};
use substrate_subxt::{balances::Balances,
                      sp_core::crypto::Ss58AddressFormat,
                      sp_runtime::traits::SignedExtension,
                      staking::{NominateCallExt, NominatorsStore, RewardDestination,
                                SetPayeeCallExt, Staking},
                      system::System,
                      Client, SignedExtra};

fn parse_reward_destination(arg: &str) -> Result<RewardDestination, &'static str> {
    Ok(match arg {
        "Staked" => RewardDestination::Staked,
        "Stash" => RewardDestination::Stash,
        "Controller" => RewardDestination::Controller,
        _ => return Err("bad reward destination ― must be Staked, Stash, or Controller"),
    })
}

#[derive(StructOpt, Debug)]
pub(crate) enum Stash {
    /// Show the specified stash controller
    Show { index: u32 },
    /// Show the status of all stash controllers
    Status,
    /// Claim a validation payout
    Claim { index: Option<u32> },
    /// Nominate a new validator set
    #[structopt(name = "nominate")]
    Nominate {
        index: u32,
        #[structopt(parse(try_from_str = parse_address))]
        set: Vec<(AccountId, u8)>,
    },
    /// Set payment target
    #[structopt(name = "set-payee")]
    SetPayee {
        index: u32,
        #[structopt(parse(try_from_str = parse_reward_destination))]
        target: RewardDestination,
    },
    /// Add a new controller key
    #[structopt(name = "add-controller-key")]
    AddControllerKey,
}

pub(crate) async fn main<
    T: System<AccountId = AccountId, Address = AccountId>
        + Balances
        + Send
        + Sync
        + Staking
        + std::fmt::Debug
        + 'static,
    S: codec::Encode + Send + Sync + 'static,
    E: SignedExtension + SignedExtra<T> + 'static,
>(
    cmd: Stash,
    client: Client<T, S, E>,
    network: Ss58AddressFormat,
    keystore: &dyn crate::keys::KeyStore<T, S, E>,
) -> Result<T::Hash, Error>
where
    <<E as SignedExtra<T>>::Extra as SignedExtension>::AdditionalSigned: Send + Sync,
{
    use std::convert::{TryFrom, TryInto};
    match cmd {
        Stash::Status => unimplemented!("showing validator status"),
        Stash::Show { index } => unimplemented!("retrieving stash keys"),
        Stash::Claim { index } => unimplemented!("claiming payment for {:?}", index),
        Stash::Nominate { index, set } => {
            let path = LedgeracioPath::new(network, AccountType::Stash, index)?;
            let signer = keystore.signer(path)?;
            let mut new_set = vec![];
            for (address, provided_network) in set.into_iter() {
                if network != provided_network.try_into().unwrap() {
                    return Err(format!(
                        "Network mismatch: address {} is for network {}, but you asked to use \
                         network {}",
                        address,
                        String::from(Ss58AddressFormat::try_from(provided_network).unwrap()),
                        String::from(Ss58AddressFormat::try_from(network).unwrap()),
                    )
                    .into())
                }
                new_set.push(address)
            }
            Ok(client.nominate(&*signer, new_set).await?)
        }
        Stash::SetPayee { index, target } => {
            let path = LedgeracioPath::new(network, AccountType::Stash, index)?;
            let signer = keystore.signer(path)?;
            Ok(client.set_payee(&*signer, target).await?)
        }
        Stash::AddControllerKey => unimplemented!("adding a controller key"),
    }
}
