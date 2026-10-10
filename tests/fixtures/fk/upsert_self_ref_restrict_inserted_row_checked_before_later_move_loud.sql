-- `upsert_self_ref_inserted_row_checked_before_later_move_loud` under `ON UPDATE RESTRICT`:
-- the inserted row's queued check refuses `23503` before the later row's `RESTRICT` action
-- can refuse `23001`.
-- @setup
CREATE TABLE p (id int PRIMARY KEY, par int REFERENCES p ON UPDATE RESTRICT);
INSERT INTO p VALUES (1, NULL);
-- @test
-- @expect-error: the inserted row's check before the RESTRICT action
-- @expect-message
-- @profile-lock
INSERT INTO p VALUES (4, 1), (1, NULL) ON CONFLICT (id) DO UPDATE SET id = 10;
