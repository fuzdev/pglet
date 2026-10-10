-- `NULLIF` resolves an `=` operator rather than a common type, so a computed text
-- argument beside the enum is `42883` where the four value constructors are `42804`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT nullif(m, lower(s)) FROM t;
