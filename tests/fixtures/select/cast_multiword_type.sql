-- @setup
-- A `CAST`/`::` to a multi-word or length-bearing type now parses: the CAST path
-- routes through the same type-name reader as the DDL column path, so `double
-- precision`, `varchar(255)`, and `timestamp with time zone` resolve (the old
-- single-identifier reader left the trailing word / length behind and failed).
CREATE TABLE castt (id INT, raw TEXT);
INSERT INTO castt VALUES (1, '1.5'), (2, '42');

-- @test
SELECT id,
       CAST(raw AS double precision) AS dp,
       raw::varchar(255) AS vc,
       CAST('2024-06-15T12:30:45Z' AS timestamp with time zone) AS ts
FROM castt
ORDER BY id;
