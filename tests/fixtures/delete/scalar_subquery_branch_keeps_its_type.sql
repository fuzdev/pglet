-- It writes: the correlated `numeric` subquery keeps `COALESCE` `numeric`, so only the
-- row `1.25` equals `1.25`.
-- @setup
CREATE TABLE ssbd (n NUMERIC);
INSERT INTO ssbd VALUES (1.25), (1);
-- @test
DELETE FROM ssbd WHERE coalesce((SELECT ssbd.n), 2) = 1.25 RETURNING n;
