-- The actions of a list run in order, so naming one constraint twice drops it once and
-- then finds it missing: `42704`.
-- @expect-error: the second drop finds the constraint gone
-- @expect-message
-- @setup
CREATE TABLE dctw (a INT CHECK (a > 0));

-- @test
ALTER TABLE dctw DROP CONSTRAINT dctw_a_check, DROP CONSTRAINT dctw_a_check;
