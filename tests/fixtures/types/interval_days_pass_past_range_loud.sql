-- The days pass lands on 294277-01-01, the first day past 18.6's timestamp range, and is
-- refused there; `interval '103830043 days -2491921032 hours'` is a value.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '9999-12-31' + interval '103830044 days -2491921056 hours';
