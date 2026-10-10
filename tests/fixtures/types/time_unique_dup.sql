-- A duplicate value in a UNIQUE TIME column violates the unique index — both
-- Postgres and pglet raise 23505, so the SQLSTATEs must match (exercises the TIME
-- index-key enforcement path).
-- @profile-lock
-- @setup
CREATE TABLE u (t TIME UNIQUE);
INSERT INTO u VALUES ('08:00:00');

-- @test
-- @expect-error: duplicate TIME violates UNIQUE (23505)
INSERT INTO u VALUES ('08:00:00');
