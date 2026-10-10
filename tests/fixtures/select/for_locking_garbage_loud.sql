-- An unrecognized FOR-locking strength is a syntax error, matching Postgres
-- (42601), instead of being silently swallowed to the statement boundary as the
-- old greedy token-eater did.
-- @expect-error: `FOR XYZ` is not a valid lock strength (syntax error)
-- @setup
CREATE TABLE lockg (id INT);
-- @test
SELECT * FROM lockg FOR XYZ;
