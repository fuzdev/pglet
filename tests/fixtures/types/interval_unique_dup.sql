-- A UNIQUE interval column dedups on the estimate, so `30 days` and `1 mon` collide.
-- Postgres also errors (23505); pglet's SQLSTATE must match.
-- @profile-lock
-- @setup
CREATE TABLE iv_uniq (span INTERVAL UNIQUE);
INSERT INTO iv_uniq VALUES (INTERVAL '30 days');
-- @test
-- @expect-error: 30 days collides 1 mon on the estimate
INSERT INTO iv_uniq VALUES (INTERVAL '1 mon');
