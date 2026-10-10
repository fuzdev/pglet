-- Same ordering against the relkind gate, which lives in the function body: a table
-- named with a wrongly-typed argument is 42883, not the 42809 the body would raise.
-- @setup
CREATE TABLE svk_t (a INT);
-- @test
SELECT setval('svk_t', NULL::text);
-- @expect-error: the overload is resolved before the body's kind gate (42883)
