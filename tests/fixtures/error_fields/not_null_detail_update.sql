-- The UPDATE route reports the *post-update* row, not the stored one.
-- @setup
CREATE TABLE t_nnu (id INT NOT NULL, name TEXT);
INSERT INTO t_nnu VALUES (1, 'a');

-- @test
-- @expect-error: UPDATE nulls a NOT NULL column
-- @expect-message
UPDATE t_nnu SET id = NULL;
