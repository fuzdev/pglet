-- `ON CONFLICT` inferring a partial unique index whose predicate short-circuits: the new
-- row's membership is decided by the left operand, so the arbiter sees no conflict for `b`.
-- @profile-lock
-- @setup
CREATE TABLE su (id INT, s TEXT);
CREATE UNIQUE INDEX su_u ON su (s) WHERE (id = 1 OR 1/(id-1) = 0);
INSERT INTO su VALUES (1, 'a');
-- @test
INSERT INTO su (id, s) VALUES (1, 'b'), (1, 'a') ON CONFLICT (s) WHERE (id = 1 OR 1/(id-1) = 0)
DO NOTHING RETURNING id, s;
