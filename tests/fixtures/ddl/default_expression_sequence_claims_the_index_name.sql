-- The sequence a `DEFAULT` names is in `pg_class`, so a constraint index of the same
-- statement must not be minted on top of it: `ChooseRelationName` appends a digit and
-- the index becomes `<name>1`. The claim used to read the closed `NextVal` arm alone,
-- so the expression spelling claimed nothing and the index took the sequence's name —
-- the accept-then-brick shape only `create_unique_index`'s post-mutation backstop
-- caught, after the statement had burned the table name.
-- @setup
CREATE SEQUENCE claim_t_pkey;
CREATE TABLE claim_t (a INT DEFAULT nextval('claim_t_pkey') + 1, b INT PRIMARY KEY);
-- @test
SELECT indexname FROM pg_indexes WHERE tablename = 'claim_t' ORDER BY indexname;
