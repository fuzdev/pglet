-- The before-list runs before the heap and the heap before the cooked defaults, so a
-- colliding declared `SEQUENCE NAME` on a *later* column outranks the default's own
-- complaint about an earlier one. pglet used to answer 42809, interleaving the two in
-- written order.
-- @setup
CREATE SEQUENCE dnyds_taken;
CREATE TABLE dnyds_host (a INT);
-- @test
CREATE TABLE dnyds (b INT DEFAULT nextval('dnyds_host'), a INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME dnyds_taken));
-- @expect-error: the colliding declared sequence name outranks the default (42P07)
