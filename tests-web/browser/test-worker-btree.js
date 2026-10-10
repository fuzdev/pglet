// Web Worker for B+tree OPFS persistence tests. Loads the build with the crate's `opfs`
// feature on (`deno task wasm:build:opfs`): the default build exports no `PgletBtreePersistent`.
import init, { PgletBtreePersistent } from '/crates/pglet_wasm/pkg-opfs/pglet_wasm.js';

let db = null;
let wasmInitialized = false;

async function initDB() {
    if (!wasmInitialized) {
        await init();
        wasmInitialized = true;
        console.log('[btree-worker] WASM initialized');
    }

    // Already have an open db - return success
    if (db) {
        return { success: true };
    }

    try {
        db = await PgletBtreePersistent.open();
        console.log('[btree-worker] PgletBtreePersistent opened');
        return { success: true };
    } catch (e) {
        console.error('[btree-worker] Failed to open:', e);
        return { success: false, error: String(e) };
    }
}

function execSQL(sql) {
    if (!db) return { error: 'Database not initialized' };
    try {
        db.exec(sql);
        return { success: true };
    } catch (e) {
        return { error: String(e) };
    }
}

function querySQL(sql) {
    if (!db) return { error: 'Database not initialized' };
    try {
        const result = db.query(sql);
        // Convert BigInt to Number for serialization
        const rows = result.rows.map(row => {
            const converted = {};
            for (const [key, value] of Object.entries(row)) {
                converted[key] = typeof value === 'bigint' ? Number(value) : value;
            }
            return converted;
        });
        return {
            success: true,
            result: {
                rows,
                rowCount: Number(result.rowCount),
                fields: result.fields
            }
        };
    } catch (e) {
        return { error: String(e) };
    }
}

function syncDB() {
    if (!db) return { error: 'Database not initialized' };
    try {
        db.sync();
        return { success: true };
    } catch (e) {
        return { error: String(e) };
    }
}

function clearDB() {
    if (!db) return { error: 'Database not initialized' };
    try {
        db.clear();
        db = null;  // Reset so next init() reopens
        return { success: true };
    } catch (e) {
        return { error: String(e) };
    }
}

// Handle messages from main thread
self.onmessage = async function(e) {
    const { id, type, sql } = e.data;

    let response;
    switch (type) {
        case 'init':
            response = await initDB();
            break;
        case 'exec':
            response = execSQL(sql);
            break;
        case 'query':
            response = querySQL(sql);
            break;
        case 'sync':
            response = syncDB();
            break;
        case 'clear':
            response = clearDB();
            break;
        default:
            response = { error: `Unknown message type: ${type}` };
    }

    self.postMessage({ id, ...response });
};

console.log('[btree-worker] Worker loaded, ready for messages');
