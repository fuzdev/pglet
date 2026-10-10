-- `m::text` **is** a `text` operand on both engines, so the cast that erases the identity
-- also makes this pair legal — the control that says the undecidable state is not a blanket
-- waiver of the string family's own rules.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, n mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
SELECT id, (m::text = s) AS eq, COALESCE(m::text, s) AS c FROM t ORDER BY id;
