-- The reader's raw-control gate at the `COPY` route. The field's text is
-- `{"k": "a\tb"}` with a *backslash-t*, which `CopyReadAttributesText` de-escapes to a
-- raw tab before the jsonb input function sees it -- so the reader is handed the raw
-- character even though the file holds none. 18.6 refuses the line with
-- `22P02 invalid input syntax for type json`; pglet used to write the row.
-- @setup
CREATE TABLE jc (id int, v jsonb);
-- @copy-from
COPY jc FROM STDIN;
-- @data-hex
31 09 7b 22 6b 22 3a 20 22 61 5c 74 62 22 7d 0a
-- @verify
SELECT id, v::text FROM jc;
