pub mod hotel {
    pub mod user {
        tonic::include_proto!("user");
    }
}

use std::error::Error;

use mongodb::{bson::doc, Client, Collection, Database, IndexModel};
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};

use hotel::{user, user::user_server::User};

use crate::config::UserConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    username: String,
    password: String,
}

#[derive(Clone)]
pub struct HotelManager {
    users: u32,
    _database: Database,
    collection: Collection<Account>,
}

impl HotelManager {
    pub async fn new(users: u32, db_addr: String) -> Result<Self, Box<dyn Error>> {
        let client = Client::with_uri_str(db_addr).await?;
        let database = client.database("sheraton");
        let collection = database.collection::<Account>("collection");
        collection.delete_many(doc! {}, None).await?;
        let manager = HotelManager {
            users,
            _database: database,
            collection,
        };
        log::warn!("Populating Mongodb...");
        manager.populate_mongodb().await?;
        log::warn!("Populated Mongodb");
        Ok(manager)
    }

    async fn populate_mongodb(&self) -> Result<(), Box<dyn Error>> {
        let mut users = Vec::new();
        for i in 0..self.users {
            users.push(Account {
                username: format!("Username_{}", i),
                // As in DeathStarBench: the index repeated ten times.
                password: i.to_string().repeat(10),
            });
        }
        self.collection.insert_many(users, None).await?;
        let index = IndexModel::builder().keys(doc! { "username": 1 }).build();
        self.collection.create_index(index, None).await?;
        Ok(())
    }
}

pub struct UserImpl {
    manager: HotelManager,
}

impl UserImpl {
    pub async fn new(config: UserConfig) -> Result<Self, Box<dyn Error>> {
        let manager = HotelManager::new(config.users, config.mongodb_addr.clone()).await?;
        let user = UserImpl { manager };
        Ok(user)
    }
}

#[tonic::async_trait]
impl User for UserImpl {
    async fn check_user(
        &self,
        request: Request<user::UserRequest>,
    ) -> Result<Response<user::UserResponse>, Status> {
        let request = request.into_inner();

        let success = {
            let query = doc! { "username": request.username.clone() };
            let collection = self.manager.collection.clone();
            let account =
                crate::mongo::detached(async move { collection.find_one(query, None).await })
                    .await?;
            // Valid credentials always succeed, anything else always fails.
            matches!(account, Ok(Some(a)) if a.password == request.password)
        };

        let response = user::UserResponse { success };
        Ok(Response::new(response))
    }
}
