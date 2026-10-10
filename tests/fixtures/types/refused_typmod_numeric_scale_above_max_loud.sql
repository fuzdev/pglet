-- The **scale** is a second window with its own sentence, checked after the precision's.
-- `1.5::numeric(5,1001)` answered a coincidental `22003 numeric out of range` here — the
-- value path tripping over a scale the type should have refused outright.
-- @expect-error: a NUMERIC scale above 1000
-- @expect-message
-- @test
SELECT 1.5::numeric(5,1001) AS never_reached;
