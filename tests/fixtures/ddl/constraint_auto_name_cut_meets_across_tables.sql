-- Two 60-byte table names that differ only in their last byte choose the same name for
-- each of their constraints once it is cut, and an engine-chosen name is unique across
-- the schema's constraints — so the second table's take the digit, on the label, and the
-- name is cut again around it. Whole, the pairs differ; a dump writing them whole restores
-- short into 18.6, which truncates each to its first 63 bytes.
-- @setup
CREATE TABLE cac5_tttttttttttttttttttttttttttttttttttttttttttttttttttttta (id INT PRIMARY KEY, v INT UNIQUE CHECK (v > 0) REFERENCES cac5_tttttttttttttttttttttttttttttttttttttttttttttttttttttta, CHECK (v > id));
CREATE TABLE cac5_ttttttttttttttttttttttttttttttttttttttttttttttttttttttb (id INT PRIMARY KEY, v INT UNIQUE CHECK (v > 0) REFERENCES cac5_tttttttttttttttttttttttttttttttttttttttttttttttttttttta, CHECK (v > id));
-- @test
SELECT contype::text AS contype, conname::text AS conname, length(conname::text) AS len
FROM pg_constraint WHERE conname LIKE 'cac5\_%' AND contype <> 'n' ORDER BY contype, conname;
