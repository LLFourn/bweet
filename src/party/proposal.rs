use crate::{
    bet_database::{BetDatabase, BetId, BetState},
    bitcoin::{Amount, Script},
    change::Change,
    keychain::KeyPair,
    party::Party,
};
use anyhow::anyhow;
use bdk::{
    bitcoin,
    bitcoin::{util::psbt, Denomination},
    blockchain::Blockchain,
    database::BatchDatabase,
    reqwest, FeeRate, TxBuilder,
};
type DefaultCoinSelectionAlgorithm = bdk::wallet::coin_selection::LargestFirstCoinSelection;
use core::str::FromStr;
use olivia_core::{EventId, OracleEvent, OracleInfo};
use olivia_secp256k1::{
    schnorr_fun::fun::{marker::*, Point},
    Secp256k1,
};

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Proposal {
    pub oracle: String,
    pub event_id: EventId,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    pub value: Amount,
    pub payload: Payload,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct LocalProposal {
    pub proposal: Proposal,
    pub oracle_event: OracleEvent<Secp256k1>,
    pub oracle_info: OracleInfo<Secp256k1>,
    pub keypair: KeyPair,
    pub psbt_inputs: Vec<psbt::Input>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Payload {
    pub inputs: Vec<bdk::bitcoin::OutPoint>,
    pub public_key: Point<EvenY>,
    pub change: Option<Change>,
}

impl Proposal {
    pub fn to_string(&self) -> String {
        format!(
            "PROPOSE#{}#{}#{}#{}",
            self.value
                .to_string_in(Denomination::Bitcoin)
                .trim_end_matches('0'),
            self.oracle,
            self.event_id,
            crate::encode::serialize_base2048(&self.payload)
        )
    }

    pub fn from_str(string: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut segments = string.split("#");
        if segments.next() != Some("PROPOSE") {
            return Err("not a proposal")?;
        }
        let value = Amount::from_str_in(
            segments.next().ok_or("missing amount")?,
            Denomination::Bitcoin,
        )?;
        let oracle = segments.next().ok_or("missing oralce")?.to_string();
        let event_id = EventId::from_str(segments.next().ok_or("missing event id")?)?;
        let base2048_encoded_payload = segments.next().ok_or("missing base2048 encoded data")?;
        let payload = crate::encode::deserialize_base2048(base2048_encoded_payload)?;

        Ok(Proposal {
            oracle,
            value,
            event_id,
            payload,
        })
    }
}

impl<B: Blockchain, D: BatchDatabase, BD: BetDatabase> Party<B, D, BD> {
    pub async fn make_proposal_from_url(
        &self,
        url: reqwest::Url,
        value: Amount,
    ) -> anyhow::Result<(BetId, Proposal)> {
        let oracle_id = url.host_str().unwrap().to_string();
        let oracle_event = self.get_oracle_event_from_url(url).await?;
        let oracle_info = self.save_oracle_info(oracle_id.clone()).await?;
        self.make_proposal(oracle_info, oracle_event, value)
    }

    pub fn make_proposal(
        &self,
        oracle_info: OracleInfo<Secp256k1>,
        oracle_event: OracleEvent<Secp256k1>,
        value: Amount,
    ) -> anyhow::Result<(BetId, Proposal)> {
        let event_id = &oracle_event.event.id;
        if !event_id.is_binary() {
            return Err(anyhow!(
                "Cannot make a bet on {} since it isn't binary",
                event_id
            ));
        }
        let keypair = self.keychain.keypair_for_proposal(&event_id, 0);

        let builder = TxBuilder::default()
            .fee_rate(FeeRate::from_sat_per_vb(0.0))
            .add_recipient(Script::default(), value.as_sat());

        let (psbt, txdetails) = self
            .wallet
            .create_tx::<DefaultCoinSelectionAlgorithm>(builder)?;

        assert_eq!(txdetails.fees, 0);

        let outputs = &psbt.global.unsigned_tx.output;
        let tx_inputs = psbt
            .global
            .unsigned_tx
            .input
            .iter()
            .map(|txin| txin.previous_output.clone())
            .collect();

        let psbt_inputs = psbt.inputs.clone();

        let change = if outputs.len() > 1 {
            if outputs.len() != 2 {
                return Err(anyhow!(
                    "wallet produced psbt with too many outputs: {:?}",
                    psbt
                ));
            }
            Some(
                outputs
                    .iter()
                    .find_map(|output| {
                        if output.script_pubkey != Script::default() {
                            Some(Change::new(output.value, output.script_pubkey.clone()))
                        } else {
                            None
                        }
                    })
                    .unwrap(),
            )
        } else {
            None
        };

        let proposal = Proposal {
            oracle: oracle_info.id.clone(),
            event_id: event_id.clone(),
            value: value,
            payload: Payload {
                inputs: tx_inputs,
                public_key: keypair.public_key,
                change,
            },
        };

        let local_proposal = LocalProposal {
            proposal: proposal.clone(),
            oracle_event,
            oracle_info,
            keypair,
            psbt_inputs,
        };

        let bet_id = self
            .bets_db
            .insert_bet(BetState::Proposed { local_proposal })?;

        Ok((bet_id, proposal))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use bdk::bitcoin::{hashes::Hash, Address, OutPoint, Txid};
    use olivia_secp256k1::schnorr_fun::fun::{s, G};

    #[test]
    fn to_and_from_str() {
        let forty_two = Point::<EvenY>::from_scalar_mul(G, &mut s!(42));
        let change_address =
            Address::from_str("bc1qwqdg6squsna38e46795at95yu9atm8azzmyvckulcc7kytlcckxswvvzej")
                .unwrap();
        let mut proposal = Proposal {
            oracle: "h00.ooo".into(),
            value: Amount::from_str("0.1 BTC").unwrap(),
            event_id: EventId::from_str("/random/2020-09-25T08:00:00/heads_tails?left-win")
                .unwrap(),
            payload: Payload {
                inputs: vec![
                    OutPoint::new(Txid::from_slice(&[1u8; 32]).unwrap(), 0),
                    OutPoint::new(Txid::from_slice(&[2u8; 32]).unwrap(), 1),
                ],
                public_key: forty_two,
                change: None,
            },
        };

        let encoded = proposal.to_string();
        let decoded = Proposal::from_str(&encoded).unwrap();
        assert_eq!(decoded, proposal);

        proposal.payload.change = Some(Change::new(100_000, change_address.script_pubkey()));

        let encoded = proposal.to_string();
        let decoded = Proposal::from_str(&encoded).unwrap();
        assert_eq!(decoded, proposal);
    }
}