-- A name literal whose quoted part never closes is `invalid name syntax` (42602) on
-- both engines. This is the polarity that forces the reader to be a parser rather than
-- a `strip_prefix('"')`: reading the rest verbatim would resolve a relation whose name
-- begins with a quote character, which is the defect in its original form.
-- @test
SELECT nextval('"unterminated');
-- @expect-error: invalid name syntax (42602)
