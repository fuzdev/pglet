-- A `jsonb` key under a unique index, a B-tree. The empty array sorts below every other
-- value and is in the range; the NULL key — SQL's, not `jsonb`'s `null` — is not.
-- @profile-lock
-- @setup
CREATE TABLE rk (c JSONB, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('[]', 'empty array'), ('null', 'json null'), ('1', 'one'), ('2', 'two');
-- @param $1 jsonb 2
-- @test
SELECT v FROM rk WHERE c < $1 ORDER BY v;
