-- The write consequence of the rule, and the reason it is not a display bug: a
-- parenless `char` constant that silently took `char(1)` was *truncated before it was
-- stored*, so a value one character wider than the target column was accepted instead
-- of refused. Both engines answer `22001` here; the messages name different types
-- (`EDGES.md` row 95) and `@expect-error` does not compare them.
-- @setup
CREATE TABLE cvo (c VARCHAR(1));
-- @test
-- @expect-error: value too long for type character varying(1)
INSERT INTO cvo VALUES (char 'ab');
