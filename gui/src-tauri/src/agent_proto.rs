use tonic::codegen::{http, Body, Bytes, StdError};

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RpcCommand {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub r#type: String,
    #[prost(string, tag = "10")]
    pub message: String,
    #[prost(message, repeated, tag = "11")]
    pub images: Vec<ImageContent>,
    #[prost(string, tag = "12")]
    pub streaming_behavior: String,
    #[prost(string, tag = "20")]
    pub parent_session: String,
    #[prost(string, tag = "30")]
    pub provider: String,
    #[prost(string, tag = "31")]
    pub model_id: String,
    #[prost(string, tag = "40")]
    pub level: String,
    #[prost(string, tag = "50")]
    pub mode: String,
    #[prost(string, tag = "60")]
    pub custom_instructions: String,
    #[prost(bool, tag = "70")]
    pub enabled: bool,
    #[prost(string, tag = "80")]
    pub command: String,
    #[prost(string, tag = "90")]
    pub session_path: String,
    #[prost(string, tag = "91")]
    pub session_id: String,
    #[prost(string, tag = "92")]
    pub entry_id: String,
    #[prost(string, tag = "93")]
    pub name: String,
    #[prost(string, tag = "94")]
    pub output_path: String,
    #[prost(string, tag = "95")]
    pub cwd: String,
    #[prost(string, tag = "100")]
    pub system_prompt: String,
    #[prost(string, repeated, tag = "110")]
    pub tools: Vec<String>,
    #[prost(bool, tag = "111")]
    pub no_tools: bool,
    #[prost(bool, tag = "120")]
    pub ephemeral: bool,
    #[prost(string, repeated, tag = "130")]
    pub enabled_models: Vec<String>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ImageContent {
    #[prost(string, tag = "1")]
    pub r#type: String,
    #[prost(string, tag = "12")]
    pub file_path: String,
    #[prost(oneof = "image_content::Content", tags = "10, 11")]
    pub content: Option<image_content::Content>,
}

pub mod image_content {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Content {
        #[prost(string, tag = "10")]
        Url(String),
        #[prost(string, tag = "11")]
        Base64(String),
    }
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RpcResponse {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub r#type: String,
    #[prost(string, tag = "3")]
    pub command: String,
    #[prost(bool, tag = "4")]
    pub success: bool,
    #[prost(string, tag = "5")]
    pub data: String,
    #[prost(string, tag = "6")]
    pub error: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct StreamRequest {
    #[prost(string, repeated, tag = "1")]
    pub event_types: Vec<String>,
    #[prost(string, tag = "2")]
    pub session_id: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct StreamEvent {
    #[prost(string, tag = "1")]
    pub r#type: String,
    #[prost(string, tag = "2")]
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct FutureAgentClient<T> {
    inner: tonic::client::Grpc<T>,
}

impl FutureAgentClient<tonic::transport::Channel> {
    pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<StdError>,
    {
        let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
        Ok(Self::new(conn))
    }
}

impl<T> FutureAgentClient<T>
where
    T: tonic::client::GrpcService<tonic::body::BoxBody>,
    T::Error: Into<StdError>,
    T::ResponseBody: Body<Data = Bytes> + Send + 'static,
    <T::ResponseBody as Body>::Error: Into<StdError> + Send,
{
    pub fn new(inner: T) -> Self {
        Self {
            inner: tonic::client::Grpc::new(inner),
        }
    }

    pub async fn execute_command(
        &mut self,
        request: impl tonic::IntoRequest<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        self.inner.ready().await.map_err(|error| {
            tonic::Status::unknown(format!("service was not ready: {}", error.into()))
        })?;
        let codec = tonic::codec::ProstCodec::default();
        let path = http::uri::PathAndQuery::from_static("/proto.FutureAgent/ExecuteCommand");
        self.inner.unary(request.into_request(), path, codec).await
    }

    pub async fn stream_events(
        &mut self,
        request: impl tonic::IntoRequest<StreamRequest>,
    ) -> Result<tonic::Response<tonic::codec::Streaming<StreamEvent>>, tonic::Status> {
        self.inner.ready().await.map_err(|error| {
            tonic::Status::unknown(format!("service was not ready: {}", error.into()))
        })?;
        let codec = tonic::codec::ProstCodec::default();
        let path = http::uri::PathAndQuery::from_static("/proto.FutureAgent/StreamEvents");
        self.inner
            .server_streaming(request.into_request(), path, codec)
            .await
    }
}
