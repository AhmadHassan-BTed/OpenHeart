//! Web Repository Adapter Module
//!
//! Provides isolated data structures and fetching logic for remote Git repository
//! ingestion and dynamic 14 UML diagram selection without coupling to core SCPG graph structures.

use std::collections::HashSet;

/// Bitmask and configuration options for selecting any of the 14 UML diagram types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UmlDiagramType {
    // Structural (7)
    ClassDiagram,
    ObjectDiagram,
    ComponentDiagram,
    DeploymentDiagram,
    PackageDiagram,
    CompositeStructure,
    ProfileDiagram,
    // Behavioral & Interaction (7)
    UseCaseDiagram,
    ActivityDiagram,
    StateMachine,
    SequenceDiagram,
    CommunicationDiagram,
    InteractionOverview,
    TimingDiagram,
}

impl UmlDiagramType {
    pub fn fill_all() -> HashSet<Self> {
        let mut set = HashSet::new();
        set.insert(Self::ClassDiagram);
        set.insert(Self::ObjectDiagram);
        set.insert(Self::ComponentDiagram);
        set.insert(Self::DeploymentDiagram);
        set.insert(Self::PackageDiagram);
        set.insert(Self::CompositeStructure);
        set.insert(Self::ProfileDiagram);
        set.insert(Self::UseCaseDiagram);
        set.insert(Self::ActivityDiagram);
        set.insert(Self::StateMachine);
        set.insert(Self::SequenceDiagram);
        set.insert(Self::CommunicationDiagram);
        set.insert(Self::InteractionOverview);
        set.insert(Self::TimingDiagram);
        set
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::ClassDiagram => "Class Diagram",
            Self::ObjectDiagram => "Object Diagram",
            Self::ComponentDiagram => "Component Diagram",
            Self::DeploymentDiagram => "Deployment Diagram",
            Self::PackageDiagram => "Package Diagram",
            Self::CompositeStructure => "Composite Structure Diagram",
            Self::ProfileDiagram => "Profile Diagram",
            Self::UseCaseDiagram => "Use Case Diagram",
            Self::ActivityDiagram => "Activity Diagram",
            Self::StateMachine => "State Machine Diagram",
            Self::SequenceDiagram => "Sequence Diagram",
            Self::CommunicationDiagram => "Communication Diagram",
            Self::InteractionOverview => "Interaction Overview Diagram",
            Self::TimingDiagram => "Timing Diagram",
        }
    }
}

/// Web Repository Configuration for deployed portal processing.
#[derive(Debug, Clone)]
pub struct WebRepoConfig {
    pub repo_url: String,
    pub branch: String,
    pub selected_diagrams: HashSet<UmlDiagramType>,
}

impl WebRepoConfig {
    pub fn new(repo_url: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            repo_url: repo_url.into(),
            branch: branch.into(),
            selected_diagrams: UmlDiagramType::fill_all(),
        }
    }

    pub fn with_diagrams(mut self, diagrams: HashSet<UmlDiagramType>) -> Self {
        self.selected_diagrams = diagrams;
        self
    }
}

/// Decoupled Web Repository Fetcher and SCPG Adapter.
pub struct WebRepoFetcher;

impl WebRepoFetcher {
    /// Validates a remote GitHub repository URL.
    pub fn validate_url(url: &str) -> Result<(String, String), String> {
        let trimmed = url.trim();
        if !trimmed.starts_with("https://github.com/") && !trimmed.starts_with("http://github.com/")
        {
            return Err("URL must start with https://github.com/".to_string());
        }

        let parts: Vec<&str> = trimmed
            .trim_start_matches("https://github.com/")
            .trim_start_matches("http://github.com/")
            .trim_end_matches(".git")
            .split('/')
            .collect();

        let owner = parts[0]
            .replace("..", "")
            .replace('/', "")
            .replace('\\', "");
        let repo = parts[1]
            .replace("..", "")
            .replace('/', "")
            .replace('\\', "");

        if owner.is_empty() || repo.is_empty() {
            return Err("Invalid owner or repository name".to_string());
        }

        Ok((owner, repo))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_repo_url_validation() {
        let res = WebRepoFetcher::validate_url("https://github.com/AhmadHassan-BTed/OpenHeart");
        assert!(res.is_ok());
        let (owner, repo) = res.unwrap();
        assert_eq!(owner, "AhmadHassan-BTed");
        assert_eq!(repo, "OpenHeart");
    }

    #[test]
    fn test_uml_diagram_all_count() {
        let all = UmlDiagramType::fill_all();
        assert_eq!(all.len(), 14);
    }
}
