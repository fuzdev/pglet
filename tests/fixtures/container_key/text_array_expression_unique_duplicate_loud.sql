-- A unique expression index whose key is an array built per row: `(ARRAY[b, a])`, the
-- columns out of table order.
-- @profile-lock
-- @setup
CREATE TABLE ckx_t (a TEXT, n INT, b TEXT);
CREATE UNIQUE INDEX ckx_t_u ON ckx_t ((ARRAY[b, a]));
INSERT INTO ckx_t (b, a, n) VALUES ('x', 'y', 1), ('y', 'x', 2), ('x', NULL, 3);
-- @test
-- @expect-error: a duplicate array expression key (23505)
INSERT INTO ckx_t (n, a, b) VALUES (4, 'y', 'x');
