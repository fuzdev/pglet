-- Mirrors the ISO-8601 timestamp projection the spine emits from
-- fuz_app's `iso8601_timestamp_column` (`db/sql_columns.ts`; used by
-- `query_app_settings_load_with_username` in `auth/app_settings_queries.ts`):
-- the `updated_at` timestamptz column is projected through
-- `to_char(s.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')`
-- with a table alias, so every spine read emits an identical second-precision
-- UTC ISO-8601 string on the wire. Fixed literal timestamp for determinism.
-- @profile-lock
-- @setup
CREATE TABLE app_settings (
  id          INT PRIMARY KEY,
  open_signup BOOLEAN NOT NULL DEFAULT false,
  updated_at  TIMESTAMPTZ NOT NULL,
  updated_by  UUID
);
INSERT INTO app_settings (id, open_signup, updated_at, updated_by) VALUES
  (1, true, '2026-06-19 12:30:45Z', '11111111-1111-1111-1111-111111111111');
-- @test
SELECT s.open_signup,
       to_char(s.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS updated_at,
       s.updated_by
  FROM app_settings s
  WHERE s.id = 1;
