-- The DDL-time rulebook's `IN` arm, which the plan-time walk had and this one did not:
-- one `CHECK` judged by a rule the query over the same columns is not. It **persisted** —
-- recorded here and refused there, so the catalog dumps to SQL stock 18.6 will not load.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL, CHECK (m IN (s)));
