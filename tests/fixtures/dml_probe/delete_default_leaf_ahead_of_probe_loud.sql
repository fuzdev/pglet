-- The DELETE spelling of `update_default_leaf_ahead_of_probe_loud`: 42601, not `DELETE 0`.
-- @profile-lock
-- @expect-error: DEFAULT is not allowed in this context
-- @setup
CREATE TABLE dd (a INT PRIMARY KEY, v TEXT);
INSERT INTO dd VALUES (1, 'x');
-- @test
DELETE FROM dd WHERE DEFAULT IS NULL AND a = -1;
