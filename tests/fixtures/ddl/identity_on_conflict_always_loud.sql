-- An `ON CONFLICT DO UPDATE SET id = <value>` on a GENERATED ALWAYS identity column
-- is rejected — the DO-UPDATE assignment path is still bound by the "can only be
-- updated to DEFAULT" rule (Postgres SQLSTATE 428C9). (A `BY DEFAULT` identity column
-- would allow it; only ALWAYS is guarded.)
-- @expect-error: DO UPDATE cannot set a GENERATED ALWAYS identity column
-- @profile-lock
-- @setup
CREATE TABLE t (id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name text);
INSERT INTO t (name) VALUES ('a');
-- @test
INSERT INTO t (name) VALUES ('b') ON CONFLICT (id) DO UPDATE SET id = 99;
