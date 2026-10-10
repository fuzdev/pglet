-- Each element of a `bytea[]` literal is read by the element type's input function, after
-- the array reader has taken its own backslashes out: a quoted `"\\x6162"` and an unquoted
-- `\\x6162` both reach it as `\x6162`, hex, while an unquoted `\x6162` reaches it as `x6162`.
-- @test
SELECT
  '{abcd,cafe,hello}'::bytea[] AS plain,
  '{"\\x6162",\\x6162,\x6162}'::bytea[] AS hex_and_not,
  '{"a b"," ","",NULL,"NULL"}'::bytea[] AS quoted,
  '{"\\\\141","\\\\\\\\",\\\\141}'::bytea[] AS escapes,
  '{ ab cd ,"\\x61 62"}'::bytea[] AS whitespace;
