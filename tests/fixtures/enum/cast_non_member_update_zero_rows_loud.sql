-- An `UPDATE … SET` of the cast over no row: 18.6 converts the literal in parse analysis,
-- so the refusal does not wait for a row.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
UPDATE t SET m = 'q'::mood WHERE false;
