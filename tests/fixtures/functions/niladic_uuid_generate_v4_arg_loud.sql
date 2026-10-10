-- `uuid_generate_v4` and `gen_random_uuid` share one `BuiltinFn` here, so the refusal
-- has to name the **spelled** function rather than the variant's other name. The
-- argument-less `uuid_generate_v4()` diverges (the oracle has no `uuid-ossp`
-- extension, so it is 42883 there and answers here) — this spelling does not, which is
-- what makes it the pinnable one.
-- @test
-- @expect-error: function uuid_generate_v4(integer) does not exist
SELECT uuid_generate_v4(1);
