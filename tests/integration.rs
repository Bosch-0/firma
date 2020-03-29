use bitcoin::Txid;
use bitcoin::{Address, Amount};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use firma::*;
use rand::{self, Rng};
use serde_json::{from_value, to_string_pretty, Value};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::{env, thread};
use tempdir::TempDir;

#[test]
fn integration_test() -> Result<()> {
    let mut rng = rand::thread_rng();
    let firma_exe_dir = env::var("FIRMA_EXE_DIR").unwrap_or("./target/debug/".to_string());
    let bitcoin_exe_dir = env::var("BITCOIN_EXE_DIR").unwrap();
    let bitcoin_work_dir = TempDir::new("bitcoin_test").unwrap();
    let cookie_file = bitcoin_work_dir.path().join("regtest").join(".cookie");
    let cookie_file_str = format!("{}", cookie_file.display());
    let rpc_port = 18242u16;
    let socket = format!("127.0.0.1:{}", rpc_port);
    let node_url = format!("http://{}", socket);
    let node_url_default = format!("http://{}/wallet/default", socket);

    let test = TcpStream::connect(&socket);
    assert!(test.is_err()); // check the port is not open with a previous instance of bitcoind

    let mut bitcoind = Command::new(&format!("{}/bitcoind", bitcoin_exe_dir))
        .arg(format!("-datadir={}", &bitcoin_work_dir.path().display()))
        .arg(format!("-rpcport={}", rpc_port))
        .arg("-daemon")
        .arg("-regtest")
        .arg("-listen=0")
        .spawn()
        .unwrap();

    // wait bitcoind is ready, use default wallet
    let client_default = loop {
        thread::sleep(Duration::from_millis(500));
        assert!(bitcoind.stderr.is_none());
        let client_result = Client::new(node_url.clone(), Auth::CookieFile(cookie_file.clone()));
        if let Ok(client_base) = client_result {
            if let Ok(_) = client_base.get_blockchain_info() {
                client_base.create_wallet("default", None).unwrap();
                break Client::new(
                    node_url_default.clone(),
                    Auth::CookieFile(cookie_file.clone()),
                )
                .unwrap();
            }
        }
    };

    // fund the bitcoind default wallet
    let address = client_default.get_new_address(None, None).unwrap();
    client_default.generate_to_address(101, &address).unwrap();
    let balance = client_default.get_balance(None, None).unwrap();
    assert!(balance.as_btc() > 49.9);

    // create firma 2of2 wallet
    let name_2of2 = "n2of2".to_string();
    let firma_2of2 = FirmaCommand::new(&firma_exe_dir, &name_2of2).unwrap();
    let r1 = firma_2of2.offline_random("r1").unwrap();
    let r2 = firma_2of2.offline_random("r2").unwrap();
    let xpubs = vec![
        r1.public_file.to_str().unwrap(),
        r2.public_file.to_str().unwrap(),
    ];
    let created_2of2_wallet = firma_2of2
        .online_create_wallet(&node_url, &cookie_file_str, 2, &xpubs)
        .unwrap();
    assert_eq!(&created_2of2_wallet.wallet.name, &name_2of2);

    // call qr view
    firma_2of2.offline_qr(xpubs[0], "xpub").unwrap();

    // create firma 2of3 wallet
    let name_2of3 = "n2of3".to_string();
    let firma_2of3 = FirmaCommand::new(&firma_exe_dir, &name_2of3).unwrap();
    let mut vec = vec![];
    for i in 0..3 {
        vec.push(firma_2of3.offline_random(&format!("p{}", i)).unwrap());
    }
    let xpubs_2of3: Vec<&str> = vec
        .iter()
        .map(|e| e.public_file.to_str().unwrap())
        .collect();
    let xprvs_2of3: Vec<&str> = vec
        .iter()
        .map(|e| e.private_file.to_str().unwrap())
        .collect();
    let created_2of3_wallet = firma_2of3
        .online_create_wallet(&node_url, &cookie_file_str, 2, &xpubs_2of3)
        .unwrap();
    assert_eq!(&created_2of3_wallet.wallet.name, &name_2of3);

    // create address for firma 2of2
    let address_2of2 = firma_2of2.online_get_address().unwrap().address;
    let fund_2of2 = 100_000_000;
    client_send_to_address(&client_default, &address_2of2, fund_2of2).unwrap();

    // create address for firma 2of3
    let address_2of3 = firma_2of3.online_get_address().unwrap().address;
    let fund_2of3 = 100_000_000;
    client_send_to_address(&client_default, &address_2of3, fund_2of3).unwrap();

    // generate 1 block so funds are confirmed
    client_default.generate_to_address(1, &address).unwrap();

    // check balances 2of2
    let balance_2of2 = firma_2of2.online_balance().unwrap();
    assert_eq!(fund_2of2, balance_2of2.satoshi);

    // check balances 2of3
    let balance_2of3 = firma_2of3.online_balance().unwrap();
    assert_eq!(fund_2of3, balance_2of3.satoshi);

    // create a tx from firma 2of2 wallet and send back to myself (detecting script reuse)
    let value_sent = rng.gen_range(1_000, 1_000_000);
    let recipients = vec![(address_2of2.clone(), value_sent)];
    let create_tx = firma_2of2.online_create_tx(recipients).unwrap();
    let psbt_files = cp(&create_tx.psbt_file, 2).unwrap();
    let sign_a_wrong = firma_2of2.offline_sign(&psbt_files[0], &r1.public_file.to_str().unwrap());
    match sign_a_wrong {
        Ok(_) => assert!(false),
        Err(e) => assert!(e.to_string().contains("missing field `xprv`")),
    }
    let sign_a = firma_2of2
        .offline_sign(&psbt_files[0], &r1.private_file.to_str().unwrap())
        .unwrap(); //TODO test passing public key
    assert!(sign_a.info.iter().any(|msg| msg.contains("#Address_reuse")));
    let sign_b = firma_2of2
        .offline_sign(&psbt_files[1], &r2.private_file.to_str().unwrap())
        .unwrap();
    assert_eq!(sign_a.fee.absolute, sign_b.fee.absolute);
    let sign_b_again = firma_2of2.offline_sign(&psbt_files[0], &r2.private_file.to_str().unwrap());
    match sign_b_again {
        Ok(_) => assert!(false),
        Err(e) => assert_eq!(e.to_string(), Error::AlreadySigned.to_string()),
    }
    let sent_tx = firma_2of2
        .online_send_tx(vec![&psbt_files[0], &psbt_files[1]])
        .unwrap();
    assert!(sent_tx.broadcasted);
    client_default.generate_to_address(1, &address).unwrap();
    let balance_2of2 = firma_2of2.online_balance().unwrap();
    let expected = fund_2of2 - sign_a.fee.absolute; // since sending to myself deduct just the fee
    assert_eq!(expected, balance_2of2.satoshi);

    // create a tx from firma 2of2 with rounded amount but same script types, check privacy analysis
    let value_sent = 1_000_000;
    let recipients = vec![(address_2of2.clone(), value_sent)];
    let create_tx = firma_2of2.online_create_tx(recipients).unwrap();
    assert!(create_tx.address_reused.contains(&address_2of2));
    let psbt_files = cp(&create_tx.psbt_file, 2).unwrap();
    let sign_a = firma_2of2
        .offline_sign(&psbt_files[0], &r1.private_file.to_str().unwrap())
        .unwrap();
    assert!(sign_a.info.iter().any(|msg| msg.contains("#Round_numbers")));
    assert!(!sign_a
        .info
        .iter()
        .any(|msg| msg.contains("#Sending_to_a_different_script_type")));

    // create a tx from firma 2of3 wallet and send back to bitcoind with keys 0 and 1
    let value_sent = rng.gen_range(1_000, 1_000_000);
    let recipients = vec![(address.clone(), value_sent)];
    let create_tx = firma_2of3.online_create_tx(recipients).unwrap();
    let psbt_files = cp(&create_tx.psbt_file, 2).unwrap();
    let sign_a = firma_2of3
        .offline_sign(&psbt_files[0], &xprvs_2of3[0])
        .unwrap(); //TODO passing xpub file gives misleading error
    assert!(sign_a
        .info
        .iter()
        .any(|msg| msg.contains("#Sending_to_a_different_script_type"))); // core generates a different address type
    let sign_b = firma_2of3
        .offline_sign(&psbt_files[1], &xprvs_2of3[1])
        .unwrap();
    assert_eq!(sign_a.fee.absolute, sign_b.fee.absolute);
    let sent_tx = firma_2of3
        .online_send_tx(vec![&psbt_files[0], &psbt_files[1]])
        .unwrap();
    assert!(sent_tx.broadcasted);
    client_default.generate_to_address(1, &address).unwrap();
    let balance_2of3 = firma_2of3.online_balance().unwrap();
    let expected = fund_2of3 - value_sent - sign_a.fee.absolute;
    assert_eq!(expected, balance_2of3.satoshi);

    // create a tx from firma 2of3 wallet and send back to bitcoind with keys 1 and 2
    let value_sent = rng.gen_range(1_000, 1_000_000);
    let recipients = vec![(address.clone(), value_sent)];
    let create_tx = firma_2of3.online_create_tx(recipients).unwrap();
    let psbt_files = cp(&create_tx.psbt_file, 2).unwrap();
    let sign_a = firma_2of3
        .offline_sign(&psbt_files[0], &xprvs_2of3[1])
        .unwrap();
    let sign_b = firma_2of3
        .offline_sign(&psbt_files[1], &xprvs_2of3[2])
        .unwrap();
    assert_eq!(sign_a.fee.absolute, sign_b.fee.absolute);
    let sent_tx = firma_2of3
        .online_send_tx(vec![&psbt_files[0], &psbt_files[1]])
        .unwrap();
    assert!(sent_tx.broadcasted);
    client_default.generate_to_address(1, &address).unwrap();
    let balance_2of3_2 = firma_2of3.online_balance().unwrap();
    let expected = balance_2of3.satoshi - value_sent - sign_a.fee.absolute;
    assert_eq!(expected, balance_2of3_2.satoshi);

    // create a tx from firma 2of3 wallet and send back to bitcoind with keys 0 and 2
    let value_sent = rng.gen_range(1_000, 1_000_000);
    let recipients = vec![(address.clone(), value_sent)];
    let create_tx = firma_2of3.online_create_tx(recipients).unwrap();
    let psbt_files = cp(&create_tx.psbt_file, 2).unwrap();
    let sign_a = firma_2of3
        .offline_sign(&psbt_files[0], &xprvs_2of3[0])
        .unwrap();
    let sign_b = firma_2of3
        .offline_sign(&psbt_files[1], &xprvs_2of3[2])
        .unwrap();
    assert_eq!(sign_a.fee.absolute, sign_b.fee.absolute);
    let sent_tx = firma_2of3
        .online_send_tx(vec![&psbt_files[0], &psbt_files[1]])
        .unwrap();
    assert!(sent_tx.broadcasted);
    client_default.generate_to_address(1, &address).unwrap();
    let balance_2of3_3 = firma_2of3.online_balance().unwrap();
    let expected = balance_2of3_2.satoshi - value_sent - sign_a.fee.absolute;
    assert_eq!(expected, balance_2of3_3.satoshi);

    let coins_output = firma_2of3.online_list_coins().unwrap();
    assert!(!coins_output.coins.is_empty());

    // stop bitcoind
    client_default.stop().unwrap();
    let ecode = bitcoind.wait().unwrap();
    assert!(ecode.success());

    Ok(())
}

struct FirmaCommand {
    pub exe_dir: String,
    pub work_dir: TempDir,
    pub wallet_name: String,
}

impl FirmaCommand {
    pub fn new(exe_dir: &str, wallet_name: &str) -> Result<Self> {
        let work_dir = TempDir::new(wallet_name).unwrap();
        Ok(FirmaCommand {
            exe_dir: exe_dir.to_string(),
            wallet_name: wallet_name.to_string(),
            work_dir,
        })
    }

    fn wallet_file(&self) -> String {
        format!(
            "{}/regtest/{}/descriptor.json",
            self.work_dir.path().display(),
            self.wallet_name
        )
    }

    pub fn online(&self, subcmd: &str, args: Vec<&str>) -> Result<Value> {
        let output = Command::new(format!("{}/firma-online", self.exe_dir))
            .arg("--firma-datadir")
            .arg(format!("{}", self.work_dir.path().display()))
            .arg("--network")
            .arg("regtest")
            .arg("--wallet-name")
            .arg(&self.wallet_name)
            .arg(subcmd)
            .args(args)
            .output()
            .unwrap();
        if output.status.success() {
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            Ok(value)
        } else {
            err(&String::from_utf8_lossy(&output.stderr))
        }
    }

    pub fn online_create_wallet(
        &self,
        node_url: &str,
        cookie_file: &str,
        required_sig: u8,
        xpubs: &Vec<&str>,
    ) -> Result<CreateWalletOutput> {
        let required_sig = format!("{}", required_sig);
        let mut args = vec![
            "--url",
            node_url,
            "--cookie-file",
            cookie_file,
            "-r",
            &required_sig,
        ];
        for xpub in xpubs {
            args.push("--xpub");
            args.push(xpub);
        }
        let result = self.online("create-wallet", args);
        let value = unwrap_as_json(result);
        let output = from_value(value).unwrap();
        println!("{}", to_string_pretty(&output).unwrap());
        Ok(output)
    }

    fn online_get_address(&self) -> Result<GetAddressOutput> {
        Ok(from_value(self.online("get-address", vec![]).unwrap())?)
    }

    fn online_balance(&self) -> Result<BalanceOutput> {
        Ok(from_value(self.online("balance", vec![]).unwrap())?)
    }

    fn online_list_coins(&self) -> Result<ListCoinsOutput> {
        Ok(from_value(self.online("list-coins", vec![]).unwrap())?)
    }

    fn online_create_tx(&self, recipients: Vec<(Address, u64)>) -> Result<CreateTxOutput> {
        let mut args = vec![];
        for recipient in recipients {
            args.push("--recipient".to_string());
            args.push(format!("{}:{}", recipient.0, recipient.1));
        }
        let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        let output = from_value(self.online("create-tx", args).unwrap())?;
        println!("{}", to_string_pretty(&output).unwrap());
        Ok(output)
    }

    fn online_send_tx(&self, psbts: Vec<&str>) -> Result<SendTxOutput> {
        let mut args = vec!["--broadcast"];
        for psbt in psbts {
            args.push("--psbt");
            args.push(psbt);
        }
        let value = self.online("send-tx", args).unwrap();
        Ok(from_value(value)?)
    }

    pub fn offline(&self, subcmd: &str, args: Vec<&str>) -> Result<Value> {
        let output = Command::new(format!("{}/firma-offline", self.exe_dir))
            .arg("--firma-datadir")
            .arg(format!("{}", self.work_dir.path().display()))
            .arg("--network")
            .arg("regtest")
            .arg(subcmd)
            .args(args)
            .output()
            .unwrap();
        if output.status.success() {
            let value: Value = serde_json::from_slice(&output.stdout)?;
            Ok(value)
        } else {
            err(&String::from_utf8_lossy(&output.stderr))
        }
    }

    pub fn offline_qr(&self, json_file: &str, index: &str) -> Result<()> {
        let result = self.offline("qr", vec![json_file, "--index", index]);
        assert!(result.is_err()); // TODO non json output
        Ok(())
    }

    pub fn offline_random(&self, key_name: &str) -> Result<MasterKeyOutput> {
        let result = self.offline("random", vec!["--key-name", key_name]);
        let value = unwrap_as_json(result);
        let output = from_value(value).unwrap();
        println!("{}", to_string_pretty(&output).unwrap());
        Ok(output)
    }

    pub fn offline_sign(&self, psbt_file: &str, key_file: &str) -> Result<PsbtPrettyPrint> {
        let result = self.offline(
            "sign",
            vec![
                psbt_file,
                "--key",
                key_file,
                "--total-derivations",
                "20",
                "--wallet-descriptor-file",
                &self.wallet_file(),
            ],
        );
        let value = map_json_error(result)?;
        let output = from_value(value)?;
        println!("{}", to_string_pretty(&output).unwrap());
        Ok(output)
    }
}

fn client_send_to_address(client: &Client, address: &Address, satoshi: u64) -> Result<Txid> {
    Ok(client
        .send_to_address(
            &address,
            Amount::from_sat(satoshi),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap())
}

fn cp(file: &PathBuf, copies: u8) -> Result<Vec<String>> {
    let mut vec = vec![];
    let file_str = file.to_str().unwrap();
    for i in 0..copies {
        let new = format!("{}.{}", file_str, i);
        Command::new("cp")
            .arg(&file_str)
            .arg(&new)
            .output()
            .unwrap();
        vec.push(new);
    }
    Ok(vec)
}
