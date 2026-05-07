pub mod card;
pub mod create;

pub use card::PollCard;
pub use create::PollCreate;

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct PollData {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub close_at: Option<String>,
    pub allow_multiple: bool,
    pub created_by: String,
    pub created_at: String,
    pub is_active: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct PollResultsData {
    pub poll: PollData,
    pub votes: Vec<PollOptionResultData>,
    pub total_votes: u32,
    pub my_votes: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct PollOptionResultData {
    pub option_index: u32,
    pub label: String,
    pub count: u32,
}
