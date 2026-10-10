-- `CREATE UNIQUE INDEX` over rows that are one `jsonb` key at two scales: the build
-- refuses, naming the key as `jsonb`'s output function writes the first of them.
-- @profile-lock
-- @setup
CREATE TABLE ckb_j (id INT, j JSONB);
INSERT INTO ckb_j (j, id) VALUES ('1', 1), ('2', 2), ('1.0', 3);
-- @test
-- @expect-error: the build finds a scale-folded duplicate (23505)
-- @expect-message
CREATE UNIQUE INDEX ckb_j_u ON ckb_j (j);
