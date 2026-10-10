-- A `DO` nested in another is one of its statements, so a counter cell the inner one
-- replaced and then **completed** is among the cells the outer `DO`'s undo puts back:
-- the inner `DO` hands the swaps it recorded up to the outer one's list
-- (`QueryResult::swapped_sequences`), since no plan can list them ahead. Here the outer
-- body fails after the inner restart and its handler catches, so the handler's draw
-- reads the old cell as the three in-place draws left it — `4` on 18.6. Handed nothing,
-- the outer undo left the inner restart in place and the handler drew `500`.
-- @setup
CREATE SEQUENCE dcn_s;
CREATE TABLE dcn_lg (v BIGINT);
INSERT INTO dcn_lg VALUES (nextval('dcn_s')), (nextval('dcn_s')), (nextval('dcn_s'));
DO $o$ BEGIN
    DO $i$ BEGIN ALTER SEQUENCE dcn_s RESTART WITH 500; END $i$;
    INSERT INTO dcn_nosuch VALUES (1);
EXCEPTION WHEN OTHERS THEN
    INSERT INTO dcn_lg VALUES (nextval('dcn_s'));
END $o$;
-- @test
SELECT v FROM dcn_lg ORDER BY v;
