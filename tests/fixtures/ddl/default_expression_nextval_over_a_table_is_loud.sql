-- pglet's **documented DDL-time over-rejection**, reached through the expression
-- spelling. `nextval` opens its relation through `init_sequence`, so a name a *table*
-- holds is `42809 cannot open relation` — on 18.6 at the first write, and here at the
-- DDL, because pglet has no `regclass` and auto-creates the named sequence, so
-- deferring would put one name in `tables` and `sequences` at once. 18.6 accepts this
-- `CREATE TABLE`, which is why any pglet error passes.
-- The bare spelling has answered this since the gate existed; the expression spelling
-- reached neither the gate nor the registration and was accepted outright.
-- @setup
CREATE TABLE nx_table (x INT);
-- @expect-error: nextval over a table is 42809 (at DDL here, at the first write on 18.6)
-- @test
CREATE TABLE nx_host (id INT DEFAULT nextval('nx_table') + 1, v INT);
