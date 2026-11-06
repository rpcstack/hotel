pub mod hotel_tonic {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }
    pub mod search {
        tonic::include_proto!("search");
    }
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
    pub mod profile {
        tonic::include_proto!("profile");
    }
    pub mod user {
        tonic::include_proto!("user");
    }
    pub mod review {
        tonic::include_proto!("review");
    }
    pub mod recommendation {
        tonic::include_proto!("recommendation");
    }
    pub mod rate {
        tonic::include_proto!("rate");
    }
}

use crate::config::HotelConfig;
// use hotel_tonic::review::review_client::ReviewClient;

use tonic::transport::Channel;
use tonic::{Request, Response, Status};

use hotel_tonic::{
    frontend, frontend::frontend_server::Frontend, profile, profile::profile_client::ProfileClient,
    rate, rate::rate_client::RateClient, recommendation,
    recommendation::recommendation_client::RecommendationClient, reservation,
    reservation::reservation_client::ReservationClient, search,
    search::search_client::SearchClient, user, user::user_client::UserClient,
};

/// No policy reports per-service queueing latency; the response fields stay 0.
fn queueing_latency<T>(_response: &Response<T>) -> u64 {
    0
}

pub struct FrontendImpl {
    search_client: SearchClient<Channel>,
    reservation_client: ReservationClient<Channel>,
    profile_client: ProfileClient<Channel>,
    user_client: UserClient<Channel>,
    recommendation_client: RecommendationClient<Channel>,
    rate_client: RateClient<Channel>,
    // review_client: ReviewClient<Channel>,
}

impl FrontendImpl {
    pub async fn new(config: HotelConfig) -> Self {
        let channel = hotel::connect(
            config.search.ip.clone(),
            config.search.port,
            config.search.replicas,
        )
        .await;
        let search_client = SearchClient::new(channel);

        let channel = hotel::connect(
            config.reservation.ip.clone(),
            config.reservation.port,
            config.reservation.replicas,
        )
        .await;
        let reservation_client = ReservationClient::new(channel);

        let channel = hotel::connect(
            config.profile.ip.clone(),
            config.profile.port,
            config.profile.replicas,
        )
        .await;
        let profile_client = ProfileClient::new(channel);

        let channel = hotel::connect(
            config.user.ip.clone(),
            config.user.port,
            config.user.replicas,
        )
        .await;
        let user_client = UserClient::new(channel);

        let channel = hotel::connect(
            config.recommendation.ip.clone(),
            config.recommendation.port,
            config.recommendation.replicas,
        )
        .await;
        let recommendation_client = RecommendationClient::new(channel);

        let channel = hotel::connect(
            config.rate.ip.clone(),
            config.rate.port,
            config.rate.replicas,
        )
        .await;
        let rate_client = RateClient::new(channel);

        // let channel = hotel::connect(
        //     config.review.ip.clone(),
        //     config.review.port,
        //     config.review.replicas,
        // )
        // .await;
        // let review_client = ReviewClient::new(channel);

        FrontendImpl {
            search_client,
            reservation_client,
            profile_client,
            user_client,
            recommendation_client,
            rate_client,
            // review_client,
        }
    }
}

#[tonic::async_trait]
impl Frontend for FrontendImpl {
    async fn handle_ping(
        &self,
        request: Request<frontend::PingRequest>,
    ) -> Result<Response<frontend::PingResponse>, Status> {
        let request = request.into_inner();
        let response = frontend::PingResponse {
            message: request.message,
        };
        let response = Response::new(response);
        Ok(response)
    }

    async fn handle_search(
        &self,
        request: Request<frontend::SearchRequest>,
    ) -> Result<Response<frontend::SearchResponse>, Status> {
        let request = request.into_inner();

        let mut search_client = self.search_client.clone();
        let search_req = search::NearbyRequest {
            lat: request.lat,
            lon: request.lon,
            in_date: request.in_date.clone(),
            out_date: request.out_date.clone(),
        };
        let search_resp = search_client.handle_nearby(search_req).await?;
        let search_q_lat = queueing_latency(&search_resp);
        let nearby_response = search_resp.into_inner();

        let mut reservation_client = self.reservation_client.clone();
        let span_request = reservation::ReservationRequest {
            customer_name: "".into(),
            hotel_ids: nearby_response.hotel_ids.clone(),
            in_date: request.in_date,
            out_date: request.out_date,
            room_number: 1,
        };

        let span_response = reservation_client.check_availability(span_request).await?;
        let reservation_q_lat = queueing_latency(&span_response);
        let availability_response = span_response.into_inner();

        let mut profile_client = self.profile_client.clone();
        let profile_request = profile::ProfileRequest {
            hotel_ids: availability_response.hotel_ids,
            locale: request.locale.unwrap_or_else(|| "en".to_string()),
        };
        let profile_response = profile_client.get_profiles(profile_request).await?;
        let profile_q_lat = queueing_latency(&profile_response);
        let response = profile_response.into_inner();

        let mut hotels = Vec::new();
        for hotel in response.hotels {
            let addr = hotel.address.unwrap();
            hotels.push(frontend::Hotel {
                name: hotel.name,
                id: hotel.id,
                phone_number: hotel.phone_number,
                lat: addr.lat,
                lon: addr.lon,
            });
        }

        let response = frontend::SearchResponse {
            hotels,
            child_traces: Vec::new(),
            search_queueing_latency: search_q_lat,
            reservation_queueing_latency: reservation_q_lat,
            profile_queueing_latency: profile_q_lat,
        };
        Ok(Response::new(response))
    }

    async fn handle_reservation(
        &self,
        request: Request<frontend::ReservationRequest>,
    ) -> Result<Response<frontend::ReservationResponse>, Status> {
        let request = request.into_inner();

        let mut user_client = self.user_client.clone();
        let user_request = user::UserRequest {
            username: request.username,
            password: request.password,
        };
        let user_response = user_client.check_user(user_request).await?;
        let user_q_lat = queueing_latency(&user_response);
        let response = user_response.into_inner();

        if !response.success {
            return Ok(Response::new(frontend::ReservationResponse {
                hotels: Vec::new(),
                user_queueing_latency: user_q_lat,
                reservation_queueing_latency: 0,
                user_correct: false,
            }));
        }

        let mut reservation_client = self.reservation_client.clone();
        let reservation_request = reservation::ReservationRequest {
            customer_name: request.customer,
            hotel_ids: request.hotels,
            in_date: request.in_date,
            out_date: request.out_date,
            // The load generator always asks for one room; the HTTP frontend
            // passes `number` through. 0 (absent) means one room.
            room_number: request.num_rooms.max(1) as i32,
        };
        let reservation_response = reservation_client
            .make_reservation(reservation_request)
            .await?;
        let res_q_lat = queueing_latency(&reservation_response);
        let response = reservation_response.into_inner();

        let response = frontend::ReservationResponse {
            hotels: response.hotel_ids,
            user_queueing_latency: user_q_lat,
            reservation_queueing_latency: res_q_lat,
            user_correct: true,
        };

        Ok(Response::new(response))
    }

    // async fn handle_review(
    //     &self,
    //     request: Request<frontend::ReviewRequest>,
    // ) -> Result<Response<frontend::ReviewResponse>, Status> {
    //     let request = request.into_inner();
    //     let mut review_client = self.review_client.clone();
    //
    //     let review_req = hotel_tonic::review::ReviewRequest {
    //         hotel_id: request.hotel_id,
    //     };
    //
    //     let review_resp = review_client.get_reviews(review_req).await?;
    //     let review_resp = review_resp.into_inner();
    //
    //     // Map review::ReviewComm to frontend::ReviewComm
    //     let reviews = review_resp
    //         .reviews
    //         .into_iter()
    //         .map(|r| frontend::ReviewComm {
    //             review_id: r.review_id,
    //             hotel_id: r.hotel_id,
    //             name: r.name,
    //             rating: r.rating,
    //             description: r.description,
    //             images: r
    //                 .images
    //                 .into_iter()
    //                 .map(|img| frontend::Image {
    //                     url: img.url,
    //                     r#default: img.default,
    //                 })
    //                 .collect(),
    //         })
    //         .collect();
    //
    //     Ok(Response::new(frontend::ReviewResponse { reviews }))
    // }
    async fn handle_review(
        &self,
        _request: Request<frontend::ReviewRequest>,
    ) -> Result<Response<frontend::ReviewResponse>, Status> {
        Ok(Response::new(frontend::ReviewResponse { reviews: vec![] }))
    }

    async fn handle_user(
        &self,
        request: Request<frontend::UserRequest>,
    ) -> Result<Response<frontend::UserResponse>, Status> {
        let request = request.into_inner();
        let mut user_client = self.user_client.clone();
        let response = user_client
            .check_user(user::UserRequest {
                username: request.username,
                password: request.password,
            })
            .await?
            .into_inner();
        Ok(Response::new(frontend::UserResponse {
            correct: response.success,
        }))
    }

    async fn handle_recommendation(
        &self,
        request: Request<frontend::RecommendationRequest>,
    ) -> Result<Response<frontend::RecommendationResponse>, Status> {
        let request = request.into_inner();

        let mut recommendation_client = self.recommendation_client.clone();
        let recommended = recommendation_client
            .get_recommendations(recommendation::RecommendationRequest {
                require: request.require,
                lat: request.lat,
                lon: request.lon,
            })
            .await?
            .into_inner();

        let mut profile_client = self.profile_client.clone();
        let profiles = profile_client
            .get_profiles(profile::ProfileRequest {
                hotel_ids: recommended.hotel_ids,
                locale: request.locale.unwrap_or_else(|| "en".to_string()),
            })
            .await?
            .into_inner();

        let hotels = profiles
            .hotels
            .into_iter()
            .map(|hotel| {
                let addr = hotel.address.unwrap_or_default();
                frontend::Hotel {
                    name: hotel.name,
                    id: hotel.id,
                    phone_number: hotel.phone_number,
                    lat: addr.lat,
                    lon: addr.lon,
                }
            })
            .collect();
        Ok(Response::new(frontend::RecommendationResponse { hotels }))
    }

    async fn handle_rates(
        &self,
        request: Request<frontend::RatesRequest>,
    ) -> Result<Response<frontend::RatesResponse>, Status> {
        let hotel_id = request.into_inner().hotel_id;
        let mut rate_client = self.rate_client.clone();
        // The rate service answers with plans of the requested hotels, but on
        // a cache miss it returns every plan in its inventory (it does not
        // filter by hotel); keep only this hotel's.
        let plans = rate_client
            .get_rates(rate::RateRequest {
                hotel_ids: vec![hotel_id.clone()],
                in_date: String::new(),
                out_date: String::new(),
            })
            .await?
            .into_inner();

        let rate_plans = plans
            .rate_plans
            .into_iter()
            .filter(|plan| plan.hotel_id == hotel_id)
            .map(|plan| {
                let room = plan.room_type.unwrap_or_default();
                frontend::RatePlan {
                    code: plan.code,
                    in_date: plan.in_date,
                    out_date: plan.out_date,
                    room_type: Some(frontend::RoomType {
                        code: room.code,
                        room_description: room.room_description,
                        bookable_rate: room.bookable_rate,
                        total_rate: room.total_rate,
                        total_rate_inclusive: room.total_rate_inclusive,
                    }),
                }
            })
            .collect();
        Ok(Response::new(frontend::RatesResponse { rate_plans }))
    }
}
