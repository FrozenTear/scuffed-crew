#[allow(dead_code)]
mod announcements;
#[allow(dead_code)]
mod audit_log;
#[allow(dead_code)]
mod dashboard;
#[allow(dead_code)]
mod games;
#[allow(dead_code)]
mod members;
#[allow(dead_code)]
mod teams;
#[allow(dead_code)]
mod schedule;
#[allow(dead_code)]
mod applications;
#[allow(dead_code)]
mod matches;
#[allow(dead_code)]
mod settings;
#[allow(dead_code)]
mod tournaments;

pub use announcements::AnnouncementsPage;
pub use audit_log::AuditLogPage;
pub use dashboard::DashboardPage;
pub use games::GamesPage;
pub use members::MembersPage;
pub use teams::TeamsPage;
pub use schedule::SchedulePage;
pub use applications::ApplicationsPage;
pub use matches::MatchesPage;
pub use settings::SettingsPage;
pub use tournaments::TournamentsPage;
