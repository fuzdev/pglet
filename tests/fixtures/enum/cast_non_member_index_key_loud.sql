-- An index key expression holding the cast; its `indexdef` rendered `(m = 'q'::text)`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
CREATE INDEX t_ix ON t ((m = 'q'::mood));
