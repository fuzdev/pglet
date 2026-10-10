-- Function resolution runs before the name literal is coerced to `regclass`, so a
-- wrongly-typed argument outranks even a missing relation — the reverse of
-- `setval_missing_relation_beats_argument_error`, where the argument only fails at
-- *evaluation*.
-- @test
SELECT setval('svo_nothing', NULL::text);
-- @expect-error: the overload is resolved before the relation (42883)
