-- Bootstrap SurrealDB for the Scuffed Crew deployment.
-- Run once with root credentials after first starting SurrealDB.
--
-- Usage: surreal import --conn http://localhost:8000 \
--          --user root --pass <root-pass> \
--          scripts/bootstrap-db.surql

-- Create namespace
DEFINE NAMESPACE IF NOT EXISTS scuffed_crew;
USE NS scuffed_crew;

-- Create default database
DEFINE DATABASE IF NOT EXISTS main;
USE DB main;

-- Create a database-scoped editor user for the application.
-- Replace the password with a strong, unique secret in production.
DEFINE USER IF NOT EXISTS app ON DATABASE PASSWORD 'CHANGE_ME' ROLES EDITOR;
