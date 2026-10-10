-- A source that is not a string converts to no enum on 18.6 (`42846 cannot cast type
-- integer to mood`); it answered its digits as a label. The code is 18.6's here, raised on
-- the value rather than in parse analysis.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: no conversion from integer to mood
SELECT 5::mood;
