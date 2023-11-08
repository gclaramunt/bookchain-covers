use blockfrost::{load, BlockFrostApi, IpfsApi, IpfsSettings};
use serde::Deserialize;
use std::hash::Hasher;
use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    fs::{self},
    num::ParseIntError,
    path::Path,
};

fn build_bf_api() -> blockfrost::Result<BlockFrostApi> {
    let configurations = load::configurations_from_env()?;
    let project_id = configurations["project_id"].as_str().unwrap();
    let api = BlockFrostApi::new(project_id, Default::default());
    Ok(api)
}

fn build_ipfs() -> blockfrost::Result<IpfsApi> {
    let configurations = load::configurations_from_env()?;
    let project_id = configurations["ipfs_project_id"].as_str().unwrap();
    let api = IpfsApi::new(project_id, IpfsSettings::new());
    Ok(api)
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let api = build_bf_api().map_err(|err| err.to_string())?;

    let ipfs = build_ipfs().map_err(|err| err.to_string())?;

    let collection_ids = collections().await?;

    let mut image_hashes = HashSet::new();

    let policy_id = "1ec6e39b4eb6cfd8054d99c5870a2b37f65bea49b78a30c6038ec572";
    if collection_ids.contains(policy_id) {
        let mut assets = api
            .assets_policy_by_id(policy_id)
            .await
            .map_err(|err| err.to_string())?;
        assets.truncate(10);
        println!("{:#?}", assets);
        for asset in assets {
            let qty: i32 = asset
                .quantity
                .parse()
                .map_err(|err: ParseIntError| err.to_string())?;
            if qty > 0 {
                if !Path::new(&asset.asset).exists() {
                    let asset_details = api
                        .assets_by_id(&asset.asset)
                        .await
                        .map_err(|err| err.to_string())?;
                    println!("{:#?}", asset_details);
                    let o_path = asset_details.onchain_metadata.and_then(|json| {
                        let path = json["files"][0]["src"].as_str().map(|str| str.to_string());
                        return path;
                    });
                    match o_path {
                        Some(path) => {
                            //drop the "ipfs://" from the path
                            let mut ipfs_id: String = path.clone();
                            ipfs_id.drain(0..7);

                            let asset_data = ipfs
                                .gateway(&ipfs_id)
                                .await
                                .map_err(|err| err.to_string())?;

                            let temp_filename = asset.asset.clone() + ".tmp";

                            //skip writting if we already have the image ()
                            let hash = calculate_hash(&asset_data);
                            if !(image_hashes.contains(&hash)) {
                                //write the data to a temp file and rename to
                                fs::write(&temp_filename, asset_data)
                                    .map_err(|err| err.to_string())?;
                                fs::rename(&temp_filename, &asset.asset)
                                    .map_err(|err| err.to_string())?;
                                image_hashes.insert(hash);
                            }
                        }
                        None => {}
                    }
                }
            }
        }
    } else {
        print!("invalid policy id {:#?}", policy_id);
    }

    Ok(())
}

fn calculate_hash(t: &Vec<u8>) -> u64 {
    let mut s = DefaultHasher::new();
    s.write(t);
    return s.finish();
}

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
    let response = client
        .get(request_url)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    // Check if the request was successful
    if response.status().is_success() {
        // Parse the JSON response into your struct
        let parsed_data: CollectionsResponse =
            response.json().await.map_err(|err| err.to_string())?;
        let id_vec = parsed_data.data.iter().map(|de| de.collection_id.clone());
        let set_data: HashSet<String> = id_vec.into_iter().collect();
        return Ok(set_data);
    } else {
        return Ok(HashSet::new());
    }
}
