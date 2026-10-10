-- The mirror, and the reason the rule is source order rather than a pass ordering: the
-- unbindable name now comes first and takes the statement. No pair of whole-tree passes
-- gives both this and its sibling fixture.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: item one's unbound name outranks the operand gate at item two
-- @expect-message
SELECT nosuchcol, m > lower(s) FROM t;
