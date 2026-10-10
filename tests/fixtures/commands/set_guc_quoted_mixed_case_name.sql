-- A setting's name is matched case-insensitively whatever its quoting, as 18.6's
-- `find_option` matches it: `SET "LOCK_TIMEOUT"` and `RESET "Lock_Timeout"` name the setting
-- `lock_timeout` names, and `SET "Search_Path" = public` is accepted as the bare spelling is. A
-- quoted name read verbatim reached none of them — each statement answered `SET` / `RESET` and
-- changed nothing, the unmodelled-name echo — and a check keyed on the name was skipped with it
-- (`set_guc_quoted_mixed_case_*_loud.sql`).
-- @setup
-- (a first statement that is not a `SET`: the header above reaches the setup's first
-- statement, and a comment-prefixed `SET` is row 23's classifier gap)
SELECT 1;
SET "Search_Path" = public;
SET "LOCK_TIMEOUT" = '3s';
DROP TABLE IF EXISTS guc_case_seen;
CREATE TABLE guc_case_seen (at TEXT, v TEXT);
INSERT INTO guc_case_seen SELECT 'set', current_setting('lock_timeout');
SET lock_timeout = '4s';
RESET "Lock_Timeout";
INSERT INTO guc_case_seen SELECT 'reset leaves 4s', (current_setting('lock_timeout') = '4s')::text;
-- @test
SELECT at, v FROM guc_case_seen ORDER BY at;
