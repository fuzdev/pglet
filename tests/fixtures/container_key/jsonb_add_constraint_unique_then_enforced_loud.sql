-- `ALTER TABLE … ADD CONSTRAINT … UNIQUE` over existing distinct `jsonb` rows builds
-- the index over them, and it then refuses a duplicate of a row that was there first.
-- @profile-lock
-- @setup
CREATE TABLE cky_j (id INT, j JSONB);
INSERT INTO cky_j (j, id) VALUES ('{"a":1}', 1), ('{"a":2}', 2), ('[1,2]', 3);
ALTER TABLE cky_j ADD CONSTRAINT cky_j_u UNIQUE (j);
-- @test
-- @expect-error: a duplicate of a back-filled key (23505)
-- @expect-message
INSERT INTO cky_j (j, id) VALUES ('[1,2.0]', 4);
