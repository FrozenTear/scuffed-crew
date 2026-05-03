mod home;
mod identity;
mod community;
mod feed;
mod members;
mod member_profile;
mod news;
mod apply;
mod tournaments;
mod tournament;
mod polls;
mod not_found;

pub mod admin;
pub mod strategy;

pub use home::Home;
pub use identity::IdentitySettings;
pub use community::Community;
pub use feed::Feed;
pub use members::Members;
pub use member_profile::MemberProfile;
pub use news::News;
pub use apply::Apply;
pub use tournaments::Tournaments;
pub use tournament::Tournament;
pub use polls::Polls;
pub use not_found::NotFound;

pub use admin::*;
pub use strategy::*;
