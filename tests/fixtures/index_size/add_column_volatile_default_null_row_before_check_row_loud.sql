-- The control for `add_column_volatile_default_check_row_before_null_row_loud`: the NULL in
-- the first row now, and the CHECK failing in none — `23502`.
-- @setup
CREATE TABLE t (id int);
INSERT INTO t VALUES (1), (2);
CREATE SEQUENCE s;
-- @test
-- @expect-error: the first row's NULL
-- @expect-message
-- @profile-lock
ALTER TABLE t ADD COLUMN b int NOT NULL CHECK (b > 1) DEFAULT NULLIF(nextval('s'), 1);
