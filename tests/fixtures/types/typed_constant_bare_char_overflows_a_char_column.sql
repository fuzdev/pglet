-- The same refusal with a `CHAR(1)` target rather than a `VARCHAR(1)` one. **The length
-- gate it reaches is not a second one**: under the `char`/`varchar` fold both column
-- spellings resolve to the identical descriptor (`TypeOid::Varchar` +
-- `varchar_length = Some(1)`) and raise the identical message, so a regression *in the
-- gate* moves this and `typed_constant_bare_char_overflows_a_varchar_column.sql`
-- together. What is this fixture's alone is the `CHAR(1)` **spelling** reaching that
-- descriptor at all, and 18.6's side: that a `CHAR(1)` target there is a length gate
-- too — `22001`, not the silent blank-pad-or-truncate a fixed-length type invites the
-- reader to expect — and that the SQLSTATEs agree across the type-name gap. Both
-- engines answer `22001`; 18.6 names
-- `character(1)` where pglet names `character varying(1)` (`EDGES.md` row 95), and
-- `@expect-error` compares the SQLSTATE rather than the message.
-- @setup
CREATE TABLE cco (c CHAR(1));
-- @test
-- @expect-error: value too long for type character(1)
INSERT INTO cco VALUES (char 'ab');
