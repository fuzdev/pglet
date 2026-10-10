-- A `SERIAL`'s engine-chosen `<table>_<column>_seq` steps around the **relation**
-- namespace only (`ChooseRelationName`), so over an enum of that name the minted name
-- is kept and the sequence's creation is `42710` — not a disambiguated `_seq1`.
-- @setup
CREATE TYPE tnt7_a_seq AS ENUM ('x');
-- @expect-error: the minted sequence name over an enum
-- @expect-message
-- @test
CREATE TABLE tnt7 (a SERIAL);
