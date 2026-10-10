-- The other order, and the control that says the precedence is **source order** rather
-- than a pass ordering: with the unbindable name first, both engines answer its `42703`.
-- The pair only discriminates together — either one alone is satisfied by a gate that
-- always wins or always loses.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
-- @expect-error: item one's unbound name outranks the conversion at item two
-- @expect-message
SELECT nosuchcol, greatest(m, 'zzz') FROM t;
