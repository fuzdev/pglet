-- The `AT TIME ZONE` half of the same control pair (see
-- `default_b_expr_excludes_at_time_zone.sql`).
-- @setup
CREATE TABLE sd_atz (k INT, b TIMESTAMP);
ALTER TABLE sd_atz ALTER COLUMN b SET DEFAULT '2024-01-01 00:00:00'::timestamptz AT TIME ZONE 'utc';
INSERT INTO sd_atz (k) VALUES (1);
-- @test
SELECT k, b FROM sd_atz;
