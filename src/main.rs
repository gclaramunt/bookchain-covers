use blockfrost::{load, AssetPolicy, BlockFrostApi};
use bytes::Bytes;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::env;
use std::error::Error;
use std::{
    collections::HashSet,
    fs::{self},
    path::Path,
};
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tokio_retry::Retry;

const BOOK_IO_COLLECTIONS_URL: &str = "https://api.book.io/api/v0/collections";

/// build Blockfrost api from configuration
fn build_bf_api() -> blockfrost::Result<BlockFrostApi> {
    let configurations = load::configurations_from_env()?;
    let project_id = configurations["project_id"].as_str().unwrap();
    let api = BlockFrostApi::new(project_id, Default::default());
    Ok(api)
}

// Simplifies passing around the configuration parameters
struct Config<'a> {
    api: &'a BlockFrostApi,
    ipfs_gateway: &'a str,
    work_dir: &'a str,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    //number of files to process on each
    let chunk_size = 10;

    //parse command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() == 1 {
        println!("Missing policy id");
        println!("Usage: \tbook_cli <policy_id> <work_dir>? <total_files>?");
        println!("\tpolicy_id (mandatory): policy id of the asset");
        println!("\twork_dir (optional): directory where to store the files (default: current directory)");
        println!("\ttotal_files (optional): maximum number of files to download (default: 10)");
        println!("\tipfs http gateway (optional): url of the ipfs gateway (default: https://ipfs.io/ipfs/)");

        return Ok(());
    }

    // load command line parameters, there's probably a rust crate that does it better
    let policy_id = &args[1];
    let work_dir: String = args.get(2).unwrap_or(&String::from(".")).to_owned();
    let max_files = args
        .get(3)
        .and_then(|max| max.parse::<u32>().ok())
        .unwrap_or(10);
    let gateway: String = args
        .get(4)
        .unwrap_or(&"https://ipfs.io/ipfs/".to_owned())
        .to_owned();

    let api = build_bf_api()?;

    let config = Config {
        api: &api,
        ipfs_gateway: &gateway,
        work_dir: &work_dir,
    };

    //read collections from book.io
    let collection_ids = collections().await?;

    //keep track of already processed files
    let mut file_hashes: HashSet<String> = HashSet::new();

    if collection_ids.contains(policy_id) {
        let mut file_count: u32 = 0;

        //read the asset's policies and process them by chunks (so we can stop when we have enough files)
        let assets = api.assets_policy_by_id(policy_id).await?;

        let chunks = assets.chunks(chunk_size);
        for chunk in chunks {
            //fetch the files for each chunk of policies
            file_count += fetch_files(
                &config,
                &mut file_hashes,
                &chunk.to_vec(),
                max_files - file_count,
            )
            .await?;

            if file_count >= max_files {
                break;
            }
        }
    } else {
        print!("invalid policy id {:#?}", policy_id);
    }

    Ok(())
}

/// fetch the files for a list of asset policies up to `files_needed` and
/// checking if the file is already present by name (uses the policy id) or by content (uses the hash and checks `file_hashes` )
async fn fetch_files<'a>(
    cfg: &Config<'a>,
    file_hashes: &mut HashSet<String>,
    assets: &Vec<AssetPolicy>,
    files_needed: u32,
) -> Result<u32, Box<dyn Error>> {
    let mut found_files = 0;
    for asset in assets {
        let temp_filename = cfg.work_dir.to_owned() + "/" + &asset.asset + ".tmp";
        let filename = cfg.work_dir.to_owned() + "/" + &asset.asset;

        let qty: i32 = asset.quantity.parse()?;

        if found_files >= files_needed {
            //stop the iteration if we have enough files
            break;
        };

        if qty > 0 {
            if !Path::new(&filename).exists() {
                let asset_details = cfg.api.assets_by_id(&asset.asset).await?;
                match get_high_res_cover_path(asset_details) {
                    Some(path) => {
                        //drop the "ipfs://" from the path
                        let mut cid: String = path.clone();
                        cid.drain(0..7);

                        // download the high-res cover from ipfs network
                        let url = cfg.ipfs_gateway.to_owned() + &cid;
                        let asset_data = download_binary(url).await?;

                        //skip writting if we already have the image
                        if !(file_hashes.contains(cid.as_str())) {
                            //write the data to a temp file and rename to final name
                            fs::write(&temp_filename, asset_data)
                                .and_then(|_| fs::rename(&temp_filename, &filename))?;
                            file_hashes.insert(cid.to_owned());
                            found_files += 1;
                        } else {
                            println!(
                                "High-res cover {:#?} for asset {:#?} is the same as existing one",
                                path, asset.asset
                            );
                        }
                    }
                    None => {
                        println!("Asset without high-res cover image: {:#?}", asset);
                    }
                }
            } else {
                println!("Asset {:#?} already downloaded", asset.asset);

                //calculate the hash so we don't download it again under a different name
                let file_data = fs::read(filename)?;
                let hash = calculate_cid(&file_data);
                file_hashes.insert(hash);

                found_files += 1;
            }
        }
    }
    return Ok(found_files);
}

/// Downloads a binary file from an url with exponential backoff retry
async fn download_binary(url: String) -> Result<Bytes, reqwest::Error> {
    let retry_strategy = ExponentialBackoff::from_millis(10)
        .map(jitter) // add jitter to delays
        .take(3); // limit to 3 retries
    let content = Retry::spawn(retry_strategy, || reqwest::get(url.to_owned()))
        .await?
        .bytes()
        .await;
    content
}

///hash using sha2-256 (same as ipfs)
fn calculate_cid(t: &Vec<u8>) -> String {
    let mut s = Sha256::new();
    s.update(t);
    return String::from_utf8_lossy(&s.finalize()[..]).to_string();
}

///Extracts the high-res cover path from the asset's onchain metadata
fn get_high_res_cover_path(asset_details: blockfrost::AssetDetails) -> Option<String> {
    let o_path = asset_details.onchain_metadata.and_then(|json| {
        let path = json["files"][0]["src"].as_str().map(|str| str.to_owned());
        println!(
            "Found high-res cover for {:#?}",
            json["name"].as_str().unwrap_or("<Unknown>")
        );
        return path;
    });
    o_path
}

//structs representing book.io json response
#[derive(Debug, Deserialize)]
struct CollectionsResponse {
    #[serde(rename = "type")]
    data_type: String,
    data: Vec<DataEntry>,
}

#[derive(Debug, Deserialize)]
struct DataEntry {
    collection_id: String,
    description: String,
    blockchain: String,
    network: String,
}

/// Fetchs the full list of policies from book.io
async fn collections() -> Result<HashSet<String>, reqwest::Error> {
    let client = reqwest::Client::new();
    //to policy_id set

    // Send the GET request
    let response = client.get(BOOK_IO_COLLECTIONS_URL).send().await?;

    // Check if the request was successful
    if response.status().is_success() {
        // Parse the JSON response into your struct
        let parsed_data: CollectionsResponse = response.json().await?;
        let id_vec = parsed_data.data.iter().map(|de| de.collection_id.clone());
        let set_data: HashSet<String> = id_vec.into_iter().collect();
        return Ok(set_data);
    } else {
        return Ok(HashSet::new());
    }
}
