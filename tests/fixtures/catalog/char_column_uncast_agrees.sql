-- A `"char"` catalog column read **without** a cast, which every other fixture over one
-- spells `::text`. The binary leg is the subject: 18.6 sends the single byte and pglet
-- sends the value's UTF-8 (a `"char"` is a `Value::Text` here), so this is the one place
-- the two wire forms of a `"char"` are compared — the empty one (`attidentity` on a
-- plain column) included, whose byte is `\0` there and no bytes here.
-- @setup
CREATE TABLE char_raw (id INT GENERATED ALWAYS AS IDENTITY, code TEXT NOT NULL);

-- @test
SELECT a.attname::text AS attname, a.attidentity, c.relkind, t.typcategory
FROM pg_attribute a
JOIN pg_class c ON c.oid = a.attrelid
JOIN pg_type t ON t.oid = a.atttypid
WHERE c.relname::text = 'char_raw' AND a.attnum > 0
ORDER BY a.attnum
