-- `ALTER TABLE … ADD CONSTRAINT … UNIQUE` over rows that are one `jsonb` key at two
-- scales.
-- @profile-lock
-- @setup
CREATE TABLE ckt_j (id INT, j JSONB);
INSERT INTO ckt_j (j, id) VALUES ('{"a":1}', 1), ('{"a":2}', 2), ('{"a":1.0}', 3);
-- @test
-- @expect-error: the constraint's build finds a scale-folded duplicate (23505)
-- @expect-message
ALTER TABLE ckt_j ADD CONSTRAINT ckt_j_u UNIQUE (j);
