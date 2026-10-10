-- A `varchar(n)[]` column enforces its element cap on store, element-wise and with
-- the assignment context's rules — a non-blank over-run is `22001`, not a silent
-- truncation and not the whole value stored. This is the write the dropped modifier
-- used to allow.
-- @setup
CREATE TABLE tmva (a varchar(3)[]);
-- @expect-error: value too long for type character varying(3)
-- @test
INSERT INTO tmva VALUES ('{abcdefg,xy}');
