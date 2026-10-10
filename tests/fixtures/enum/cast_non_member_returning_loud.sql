-- A `RETURNING` item: the row was inserted and the label returned, where 18.6 refuses the
-- statement whole.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
INSERT INTO t VALUES (3, 'ok') RETURNING 'q'::mood;
