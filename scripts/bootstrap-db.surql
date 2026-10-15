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

-- Namespace and database only. The server creates the database user at
-- boot from SURREALDB_APP_USER and SURREALDB_APP_PASSWORD. This file
-- does not define a user or a password.
