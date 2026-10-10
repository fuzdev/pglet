-- `ALTER TABLE … ADD PRIMARY KEY` over a `jsonb` column with rows in it.
-- @profile-lock
-- @setup
CREATE TABLE ckh_j (id INT, j JSONB);
INSERT INTO ckh_j (j, id) VALUES ('"a"', 1), ('"b"', 2);
ALTER TABLE ckh_j ADD PRIMARY KEY (j);
-- @test
-- @expect-error: a duplicate of a back-filled primary key (23505)
-- @expect-message
INSERT INTO ckh_j (j, id) VALUES ('"a"', 3);
