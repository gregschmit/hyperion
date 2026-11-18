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

CREATE TABLE "categories" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "name" varchar(255) NOT NULL UNIQUE
);

CREATE TABLE "configurations" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "name" varchar NOT NULL,
  "active" boolean NOT NULL DEFAULT FALSE,
  "prompt" text NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX idx_configurations_one_active ON configurations (active) WHERE active;

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

CREATE TABLE "runs" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "operation" varchar NOT NULL,
  "repo" varchar NOT NULL,
  "status" varchar NOT NULL DEFAULT 'queued',
  "prompt" text NOT NULL DEFAULT '',
  "output" text NOT NULL DEFAULT '',
  "error" text NOT NULL DEFAULT '',
  "started_at" timestamp_with_timezone_text NULL,
  "finished_at" timestamp_with_timezone_text NULL,
  "command" varchar NOT NULL DEFAULT '',
  schedule_id bigint REFERENCES schedules (id) ON DELETE SET NULL
);
CREATE INDEX "idx_runs_status" ON "runs" ("status");

CREATE TABLE "schedules" (
  "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "operation" varchar NOT NULL,
  "repos" varchar NOT NULL,
  "hron" varchar NOT NULL,
  "command" varchar NOT NULL DEFAULT '',
  "prompt" text NOT NULL DEFAULT '',
  category_id bigint REFERENCES categories (id) ON DELETE RESTRICT
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
  "category_id" integer NOT NULL,
  "priority" varchar NOT NULL DEFAULT 'medium',
  "note" varchar NOT NULL DEFAULT '',
  "review_requested" boolean NOT NULL DEFAULT FALSE,
  "title" varchar NOT NULL DEFAULT '',
  "checked_at" timestamp_with_timezone_text NULL,
  FOREIGN KEY ("category_id") REFERENCES "categories" ("id") ON DELETE RESTRICT ON UPDATE CASCADE
);
