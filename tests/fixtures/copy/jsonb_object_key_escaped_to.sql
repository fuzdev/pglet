-- `COPY … TO STDOUT` renders each value through the same text encoder the dump's payload
-- uses, so a jsonb object key goes out with its escapes. Written raw instead, the first
-- row's payload is not valid JSON and neither engine reads the file back; the second
-- row's is another key's spelling, which both engines read back *without* an error --
-- silently a different document. Row 3 is the control.
-- @setup
CREATE TABLE jt (id int, v jsonb);
INSERT INTO jt VALUES
  (1, '{"a\"b": 1}'),
  (2, '{"a\\b": 2}'),
  (3, '{"a\tb": 3}'),
  (4, '{"ab": 4}');
-- @copy-to
COPY jt TO STDOUT;
