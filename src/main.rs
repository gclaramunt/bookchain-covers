use blockfrost::{load, AssetPolicy, BlockFrostApi};
use bytes::Bytes;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::env;
use std::{
    collections::HashSet,
    fs::{self},
    path::Path,
};

fn build_bf_api() -> blockfrost::Result<BlockFrostApi> {
    let configurations = load::configurations_from_env()?;
    let project_id = configurations["project_id"].as_str().unwrap();
    let api = BlockFrostApi::new(project_id, Default::default());
    Ok(api)
}

struct Config<'a> {
    api: &'a BlockFrostApi,
    ipfs_gateway: &'a str,
    work_dir: &'a str,
}

#[tokio::main]
async fn main() -> Result<(), String> {
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
    let policy_id = &args[1];
    let work_dir: String = args.get(2).unwrap_or(&".".to_owned()).to_owned();
    let max_files = args
        .get(3)
        .and_then(|max| max.parse::<u32>().ok())
        .unwrap_or(10);

    let gateway: String = args
        .get(4)
        .unwrap_or(&"https://ipfs.io/ipfs/".to_owned())
        .to_owned();

    let api = build_bf_api().unwrap();

    let config = Config {
        api: &api,
        ipfs_gateway: &gateway,
        work_dir: &work_dir,
    };

    let collection_ids = collections().await?;

    let mut file_hashes: HashSet<String> = HashSet::new();

    if collection_ids.contains(policy_id) {
        let mut file_count: u32 = 0;
        let assets = api.assets_policy_by_id(policy_id).await.unwrap();

        let chunks = assets.chunks(chunk_size);
        for chunk in chunks {
            file_count += fetch_files(
                &config,
                &mut file_hashes,
                &chunk.to_vec(),
                max_files - file_count,
            )
            .await;

            if file_count >= max_files {
                break;
            }
        }
    } else {
        print!("invalid policy id {:#?}", policy_id);
    }

    Ok(())
}

async fn fetch_files<'a>(
    cfg: &Config<'a>,
    file_hashes: &mut HashSet<String>,
    assets: &Vec<AssetPolicy>,
    files_needed: u32,
) -> u32 {
    let mut found_files = 0;
    for asset in assets {
        let temp_filename = cfg.work_dir.to_owned() + "/" + &asset.asset + ".tmp";
        let filename = cfg.work_dir.to_owned() + "/" + &asset.asset;

        let qty: i32 = asset.quantity.parse().unwrap();

        if found_files >= files_needed {
            //stop the iteration if we have enough files
            break;
        };

        if qty > 0 {
            if !Path::new(&filename).exists() {
                let asset_details = cfg.api.assets_by_id(&asset.asset).await.unwrap();
                match get_high_res_cover_path(asset_details) {
                    Some(path) => {
                        //drop the "ipfs://" from the path
                        let mut cid: String = path.clone();
                        cid.drain(0..7);

                        let url = cfg.ipfs_gateway.to_owned() + &cid;
                        let asset_data = fetch_cid(url).await.unwrap();

                        //skip writting if we already have the image
                        if !(file_hashes.contains(cid.as_str())) {
                            //write the data to a temp file and rename to final name
                            fs::write(&temp_filename, asset_data)
                                .and_then(|_| fs::rename(&temp_filename, &filename))
                                .unwrap();
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
                let file_data = fs::read(filename).unwrap();
                let hash = calculate_cid(&file_data);
                file_hashes.insert(hash);

                found_files += 1;
            }
        }
    }
    return found_files;
}

//Download asset
async fn fetch_cid(url: String) -> Result<Bytes, String> {
    let content = reqwest::get(url).await.unwrap().bytes().await.unwrap();
    Ok(content)
}

//hash using the same
fn calculate_cid(t: &Vec<u8>) -> String {
    let mut s = Sha256::new();
    s.update(t);
    return String::from_utf8_lossy(&s.finalize()[..]).to_string();
}

fn get_high_res_cover_path(asset_details: blockfrost::AssetDetails) -> Option<String> {
    let o_path = asset_details.onchain_metadata.and_then(|json| {
        let path = json["files"][0]["src"].as_str().map(|str| str.to_owned());
        println!(
            "Found high-res cover for {:#?}",
            json["name"].as_str().unwrap_or("")
        );
        return path;
    });
    o_path
}

//structs representing the json response
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

async fn collections() -> Result<HashSet<String>, String> {
    let client = reqwest::Client::new();
    //to policy_id set
    let request_url = "https://api.book.io/api/v0/collections";

    // Send the GET request
    let response = client.get(request_url).send().await.unwrap();

    // Check if the request was successful
    if response.status().is_success() {
        // Parse the JSON response into your struct
        let parsed_data: CollectionsResponse = response.json().await.unwrap();
        let id_vec = parsed_data.data.iter().map(|de| de.collection_id.clone());
        let set_data: HashSet<String> = id_vec.into_iter().collect();
        return Ok(set_data);
    } else {
        return Ok(HashSet::new());
    }
}
