-- Statement-level error precedence, which a deeper-reaching gate moves: the operand gate
-- now fires at the first target-list item, ahead of the unbindable name in the second.
-- 18.6 resolves the target list in source order too, so the two agree — and they did not
-- before, this shape answering `42703` here.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: the operand gate at item one outranks item two's unbound name
-- @expect-message
SELECT m > lower(s), nosuchcol FROM t;
