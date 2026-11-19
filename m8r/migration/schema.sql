-- What m8r's migrations make in a new SQLite database. In development, m8r writes this file
-- each time it runs its migrations: `cargo loco db migrate` does, and so does a start. Don't
-- edit it: it follows the migrations.

CREATE TABLE "artifacts" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "name" varchar NOT NULL,
  "url" varchar NOT NULL DEFAULT '',
  "run_id" integer NOT NULL,
  FOREIGN KEY ("run_id") REFERENCES "runs" ("id") ON DELETE CASCADE ON UPDATE CASCADE
);
CREATE UNIQUE INDEX "idx_artifacts_run_name" ON "artifacts" ("run_id", "name");

CREATE TABLE "errors" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "subject" varchar NOT NULL,
  "message" text NOT NULL DEFAULT '',
  "cleared_at" timestamp_with_timezone_text NULL,
  "run_id" integer NULL,
  FOREIGN KEY ("run_id") REFERENCES "runs" ("id") ON DELETE SET NULL ON UPDATE NO ACTION
);
CREATE UNIQUE INDEX idx_errors_one_open ON errors (subject) WHERE cleared_at IS NULL;

CREATE TABLE "repos" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "path" varchar NOT NULL UNIQUE,
  "url" varchar NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX idx_repos_one_url ON repos (url) WHERE url <> '';

CREATE TABLE "runs" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "name" varchar NOT NULL,
  "repo" varchar NOT NULL,
  "status" varchar NOT NULL DEFAULT 'queued',
  "prompt" text NOT NULL DEFAULT '',
  "output" text NOT NULL DEFAULT '',
  "error" text NOT NULL DEFAULT '',
  "started_at" timestamp_with_timezone_text NULL,
  "finished_at" timestamp_with_timezone_text NULL,
  "command" varchar NOT NULL DEFAULT '',
  schedule_id bigint REFERENCES schedules (id) ON DELETE SET NULL,
  repo_id bigint REFERENCES repos (id) ON DELETE SET NULL
);
CREATE INDEX "idx_runs_status" ON "runs" ("status");

CREATE TABLE "schedule_repos" (
  "schedule_id" integer NOT NULL,
  "repo_id" integer NOT NULL,
  PRIMARY KEY ("schedule_id", "repo_id"),
  FOREIGN KEY ("schedule_id") REFERENCES "schedules" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  FOREIGN KEY ("repo_id") REFERENCES "repos" ("id") ON DELETE RESTRICT ON UPDATE CASCADE
);

CREATE TABLE "schedules" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "name" varchar NOT NULL UNIQUE,
  "category" varchar NOT NULL DEFAULT '',
  "max_items" integer NOT NULL DEFAULT 0,
  "hron" varchar NOT NULL,
  "command" varchar NOT NULL DEFAULT '',
  "prompt" text NOT NULL DEFAULT ''
);

CREATE TABLE "tokens" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "name" varchar NOT NULL UNIQUE,
  "digest" varchar NOT NULL UNIQUE,
  "scope" varchar NOT NULL
);

CREATE TABLE "used_links" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "token_id" varchar NOT NULL UNIQUE,
  "expires_at" timestamp_with_timezone_text NOT NULL
);

CREATE TABLE "watchlist_items" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "url" varchar NOT NULL UNIQUE,
  "repo_id" integer NOT NULL,
  "category" varchar NOT NULL,
  "priority" varchar NOT NULL DEFAULT 'medium',
  "note" varchar NOT NULL DEFAULT '',
  "requested" boolean NOT NULL DEFAULT FALSE,
  "title" varchar NOT NULL DEFAULT '',
  "checked_at" timestamp_with_timezone_text NULL,
  FOREIGN KEY ("repo_id") REFERENCES "repos" ("id") ON DELETE RESTRICT ON UPDATE CASCADE
);
