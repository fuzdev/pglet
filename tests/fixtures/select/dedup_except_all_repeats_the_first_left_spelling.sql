-- @test
-- `EXCEPT ALL` emits a left group's surplus as copies of the group's first left row:
-- three equal `numeric`s less one leave two copies of the first spelling, whichever it is.
SELECT x FROM (VALUES (1.00::numeric), (1.0), (1.000)) a(x)
EXCEPT ALL SELECT x FROM (VALUES (1::numeric)) b(x);
