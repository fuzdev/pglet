-- The membership half at the same arm: an `IN` element is coerced to the subject's enum,
-- so a non-member element is refused even with a member element beside it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
CREATE TABLE ck (m mood NOT NULL, CHECK (m IN ('ok', 'zzz')));
