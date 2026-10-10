-- `CREATE UNIQUE INDEX` over rows holding one array key twice.
-- @profile-lock
-- @setup
CREATE TABLE ckb_i (id INT, a INT[]);
INSERT INTO ckb_i (a, id) VALUES ('{1,2}', 1), ('{2,1}', 2), ('{1,2}', 3);
-- @test
-- @expect-error: the build finds the repeated array key (23505)
-- @expect-message
CREATE UNIQUE INDEX ckb_i_u ON ckb_i (a);
