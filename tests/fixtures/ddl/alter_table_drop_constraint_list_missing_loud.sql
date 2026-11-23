-- In a list, a missing name without `IF EXISTS` fails the whole statement, wherever it
-- sits — after drops that would succeed and after an `IF EXISTS` skip alike.
-- @expect-error: the second action's constraint does not exist
-- @expect-message
-- @setup
CREATE TABLE dclm (a INT CHECK (a > 0), b INT CHECK (b > 0));

-- @test
ALTER TABLE dclm DROP CONSTRAINT dclm_a_check, DROP CONSTRAINT IF EXISTS nope, DROP CONSTRAINT dclm_nope;
