-- `currval` on a name **no relation at all** holds is 42P01, not the 55000 it raises
-- for a real sequence this session has not advanced: `'x'::regclass` is coerced at
-- parse analysis, ahead of the function body entirely. The corpus had the
-- wrong-kind (42809) and the not-yet-defined (55000) shapes but not this one, so a
-- build that resolved the name only through `schema.sequences` answered 55000 here
-- with nothing to catch it.
-- @test
SELECT currval('cvmr_no_such_relation');
-- @expect-error: relation does not exist (42P01)
