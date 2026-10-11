-- Bootstrap SurrealDB for the Scuffed Crew deployment.
-- Run once with root credentials after first starting SurrealDB.
--
-- Usage: scripts/bootstrap-db.sh [http://localhost:8000]
--        (runs this file through `surreal sql`; `surreal import` only takes exports)

-- Create namespace
DEFINE NAMESPACE IF NOT EXISTS scuffed_crew;
USE NS scuffed_crew;

-- Create default database
DEFINE DATABASE IF NOT EXISTS main;
USE DB main;

-- Create a database-scoped editor user for the application.
-- Replace the password with a strong, unique secret in production.
DEFINE USER IF NOT EXISTS app ON DATABASE PASSWORD 'CHANGE_ME' ROLES EDITOR;
