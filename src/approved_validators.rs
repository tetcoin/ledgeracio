// Copyright 2020 Parity Technologies (UK) Ltd.
// This file is part of ledgeracio.
//
// ledgeracio is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version.
//
// ledgeracio is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with ledgeracio.  If not, see <http://www.gnu.org/licenses/>.

//! Routines for handling approved validators

use super::{Error, StructOpt};
use crate::{AccountId, Ss58AddressFormat};
use std::{convert::{TryFrom, TryInto},
          fs::OpenOptions,
          io::Write,
          os::unix::fs::OpenOptionsExt,
          path::PathBuf};

const MAGIC: &[u8] = &*b"Ledgeracio Secret Key";
#[derive(StructOpt, Debug)]
pub(crate) enum ACL {
    /// Upload a new approved validator list.  This list must be signed.
    Upload { path: PathBuf },
    /// Set the validator list signing key.  This will fail if a signing key has
    /// already been set.
    SetKey {
        /// The file containing the public signing key.  You can generate this
        /// file with `ledgeracio allowlist gen-key`.
        key: PathBuf,
    },
    /// Get the validator list signing key.  This will fail unless a signing key
    /// has been set.
    GetKey,
    /// Generate a new signing key.
    GenKey {
        /// Prefix of the file to write the keys to
        ///
        /// The public key will be written to `file.pub` and the secret key
        /// to `file.sec`.
        file: PathBuf,
    },
    /// Compile the provided textual allowlist into a binary format and sign it.
    ///
    /// `secret` should be a secret signing key generated by `ledgeracio
    /// allowlist genkey`.  If you provide a public key, it will be verified
    /// to match the provided secret key.  This helps check that neither has
    /// been corrupted, and that you are using the correct secret key.
    Sign {
        /// The textual allowlist file.
        ///
        /// The textual allowlist format is very simple.  If a line is empty, or
        /// if its first non-whitespace character is `;` or `#`, it is
        /// considered a comment.  Otherwise, the line must be a valid SS58
        /// address for the provided network, except that leading and
        /// trailing whitespace are ignored.  The process of compiling
        /// an allowlist to binary format and signing it is completely
        /// deterministic.
        #[structopt(short = "f", long = "file")]
        file: PathBuf,
        /// The secret key file.
        #[structopt(short = "s", long = "secret")]
        secret: PathBuf,
        /// The output file
        #[structopt(short = "o", long = "output")]
        output: PathBuf,
        /// The nonce.  This must be greater than any nonce used previously with
        /// the same key, and is used to prevent replay attacks.
        #[structopt(short = "n", long = "nonce")]
        nonce: u32,
    },
    /// Inspect the given allowlist file and verify its signature. The output is
    /// in a format suitable for `ledgeracio sign`.
    Inspect {
        /// The binary allowlist file to read
        #[structopt(short = "f", long = "file")]
        file: PathBuf,
        /// The public key file.
        #[structopt(short = "p", long = "public")]
        public: PathBuf,
        /// The output file.  Defaults to stdout.
        #[structopt(short = "o", long = "output")]
        output: Option<PathBuf>,
    },
}

fn write(buf: &[&[u8]], path: &std::path::Path) -> std::io::Result<()> {
    let mut f = OpenOptions::new()
        .mode(0o400)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    for i in buf {
        f.write_all(i)?;
    }
    Ok(())
}

pub(crate) async fn main<T: FnOnce() -> Result<super::HardStore, Error>>(
    acl: ACL,
    hardware: T,
    network: Ss58AddressFormat,
) -> Result<(), Error> {
    use ed25519_dalek::Keypair;
    use std::fs;

    match acl {
        ACL::GetKey => {
            let s: [u8; 32] = hardware()?.get_pubkey().await?;
            println!("Public key is {}", hex::encode(s));
            Ok(())
        }
        ACL::SetKey { key } => {
            let key = ed25519_dalek::PublicKey::from_bytes(&*fs::read(key)?)?;
            hardware()?.set_pubkey(&key.as_bytes()).await
        }
        ACL::Upload { path } => {
            let allowlist = fs::read(path)?;
            hardware()?.allowlist_upload(&allowlist).await
        }
        ACL::GenKey { mut file } => {
            if file.extension().is_some() {
                return Err(format!(
                    "please provide a filename with no extension, not {}",
                    file.display()
                )
                .into())
            }
            let keypair = Keypair::generate(&mut rand::rngs::OsRng {});
            let secretkey = keypair.secret.to_bytes();
            let publickey = keypair.public.to_bytes();
            file.set_extension("pub");
            let public = format!(
                "Ledgeracio version 1 public key for network {}\n{}\n",
                match network {
                    Ss58AddressFormat::KusamaAccount => "Kusama",
                    Ss58AddressFormat::PolkadotAccount => "Polkadot",
                    _ => unreachable!("should have been rejected earlier"),
                },
                base64::encode(&publickey[..])
            );
            write(&[public.as_bytes()], &file)?;
            file.set_extension("sec");
            write(
                &[
                    MAGIC,
                    &1_u16.to_le_bytes(),
                    &[network.into()],
                    &secretkey[..],
                    &publickey[..],
                ],
                &file,
            )?;
            Ok(())
        }
        ACL::Sign {
            file,
            secret,
            output,
            nonce,
        } => {
            let file = std::io::BufReader::new(fs::File::open(file)?);
            let secret: Vec<u8> = fs::read(secret)?;
            if secret.len() != 88 {
                return Err(
                    format!("Ledgeracio secret keys are 88 bytes, not {}", secret.len()).into(),
                )
            }
            if &secret[..21] != MAGIC {
                return Err("Not a Ledgeracio secret key ― wrong magic number"
                    .to_owned()
                    .into())
            }
            if secret[21..23] != [1, 0][..] {
                return Err(format!(
                    "Expected a version 1 secret key, but got version {}",
                    u16::from_le_bytes(secret[21..23].try_into().unwrap())
                )
                .into())
            }
            if secret[23] != u8::from(network) {
                return Err(format!(
                    "Expected a key for network {}, but got a key for network {}",
                    network,
                    secret[23]
                        .try_into()
                        .unwrap_or_else(|()| Ss58AddressFormat::Custom(secret[23]))
                )
                .into())
            }

            let sk = (&ed25519_dalek::SecretKey::from_bytes(&secret[24..56])?).into();
            let pk = ed25519_dalek::PublicKey::from_bytes(&secret[56..88])?;
            let signed = crate::parser::parse::<_, AccountId>(file, network, &pk, &sk, nonce)?;
            fs::write(output, signed)?;
            Ok(())
        }
        ACL::Inspect {
            file,
            public,
            output,
        } => {
            use regex::bytes::Regex;
            use std::str;
            let file = std::io::BufReader::new(fs::File::open(file)?);
            let pk = fs::read(public)?;
            let re = Regex::new(r"^Ledgeracio version ([1-9][0-9]*) public key for network ([[:alpha:]]+)\n([[:alnum:]/+]+=)\n$").unwrap();
            let captures = re
                .captures(&pk)
                .ok_or_else(|| "Invalid public key".to_owned())?;
            let (version, network, data) = (
                str::from_utf8(&captures[1]).unwrap(),
                str::from_utf8(&captures[2]).unwrap(),
                str::from_utf8(&captures[3]).unwrap(),
            );
            if version != "1" {
                return Err("Only version 1 keys are supported".to_owned().into())
            }
            let network = Ss58AddressFormat::try_from(&*network.to_ascii_lowercase())
                .map_err(|()| format!("invalid network {}", network))?;
            let mut pk = [0_u8; 32];
            assert_eq!(
                base64::decode_config_slice(&*data, base64::STANDARD, &mut pk)?,
                pk.len()
            );
            let pk = ed25519_dalek::PublicKey::from_bytes(&pk[..])?;
            let stdout = std::io::stdout();
            let mut output = std::io::BufWriter::new(match output {
                None => Box::new(stdout.lock()) as Box<dyn std::io::Write>,
                Some(path) => Box::new(
                    OpenOptions::new()
                        .mode(0o600)
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(path)?,
                ),
            });

            for i in crate::parser::inspect::<_, AccountId>(file, network, &pk)? {
                writeln!(output, "{}", i)?;
            }
            Ok(())
        }
    }
}
