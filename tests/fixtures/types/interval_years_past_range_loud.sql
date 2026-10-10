-- A months pass past 18.6's timestamp range with nothing to bring it back.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '9999-12-31' + interval '300000 years';
