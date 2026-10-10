-- A word that is not `infinity` is still a malformed date, with the same code and
-- sentence on both engines.
-- @expect-error: invalid input syntax for type date
-- @expect-message
-- @test
SELECT 'infinit'::date;
