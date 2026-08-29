#include "dittoffi.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct autonomy_sim_ditto_peer {
    CDitto_t *ditto;
    dittoffi_sync_subscription_t **subscriptions;
    size_t subscription_count;
    size_t subscription_capacity;
} autonomy_sim_ditto_peer_t;

static char *bridge_strdup(const char *value) {
    size_t length = strlen(value) + 1;
    char *copy = malloc(length);
    if (copy != NULL) {
        memcpy(copy, value, length);
    }
    return copy;
}

static void set_literal_error(char **out_error, const char *message) {
    if (out_error != NULL) {
        *out_error = bridge_strdup(message);
    }
}

static void take_ditto_error(char **out_error, dittoffi_error_t *error) {
    if (out_error != NULL) {
        char *description = dittoffi_error_description(error);
        *out_error = bridge_strdup(description == NULL ? "unknown Ditto error" : description);
        if (description != NULL) {
            ditto_c_string_free(description);
        }
    }
    dittoffi_error_free(error);
}

static void bridge_panic_report(void *context, dittoffi_panic_t *panic) {
    (void)context;
    char *message = dittoffi_panic_message(panic);
    char *stack = dittoffi_panic_stack_trace_string(panic);
    fprintf(stderr, "dittoffi panic: %s\n%s\n",
            message == NULL ? "unknown" : message,
            stack == NULL ? "" : stack);
    if (message != NULL) {
        ditto_c_string_free(message);
    }
    if (stack != NULL) {
        ditto_c_string_free(stack);
    }
    dittoffi_panic_free(panic);
}

static void bridge_panic_context_free(void *context) {
    (void)context;
}

static int bridge_panic_context;

autonomy_sim_ditto_peer_t *autonomy_sim_ditto_peer_open(
    const char *working_directory,
    const char *database_id,
    const char *license,
    char **out_error) {
    dittoffi_panic_handler_t panic_handler = {
        &bridge_panic_context, bridge_panic_report, bridge_panic_context_free};
    dittoffi_ditto_set_panic_handler(panic_handler);
    ditto_init_sdk_version(PLATFORM_LINUX, LANGUAGE_RUST, "0.1.0");
    IdentityConfigResult_t identity =
        ditto_identity_config_make_offline_playground(database_id, 0);
    if (identity.status_code != 0 || identity.identity_config == NULL) {
        char message[96];
        snprintf(message, sizeof(message),
                 "creating offline Ditto identity failed with status %d",
                 identity.status_code);
        set_literal_error(out_error, message);
        return NULL;
    }
    dittoffi_result_CDitto_ptr_t opened = dittoffi_ditto_try_new_blocking(
        working_directory, identity.identity_config, NULL,
        TRANSPORT_CONFIG_MODE_PLATFORM_INDEPENDENT);
    if (opened.error != NULL) {
        take_ditto_error(out_error, opened.error);
        return NULL;
    }

    dittoffi_result_void_t licensed =
        dittoffi_ditto_set_offline_only_license_token_throws(opened.success, license);
    if (licensed.error != NULL) {
        take_ditto_error(out_error, licensed.error);
        ditto_free(opened.success);
        return NULL;
    }
    if (ditto_disable_sync_with_v3(opened.success) != 0) {
        set_literal_error(out_error, "Ditto rejected CRDT-v6-only synchronization mode");
        ditto_free(opened.success);
        return NULL;
    }

    autonomy_sim_ditto_peer_t *peer = calloc(1, sizeof(*peer));
    if (peer == NULL) {
        set_literal_error(out_error, "allocating Ditto peer wrapper failed");
        ditto_free(opened.success);
        return NULL;
    }
    peer->ditto = opened.success;
    return peer;
}

bool autonomy_sim_ditto_peer_subscribe(
    autonomy_sim_ditto_peer_t *peer,
    const char *query,
    char **out_error) {
    slice_ref_uint8_t no_args = {NULL, 0};
    dittoffi_result_dittoffi_sync_subscription_ptr_t result =
        dittoffi_sync_register_subscription_throws(peer->ditto, query, no_args);
    if (result.error != NULL) {
        take_ditto_error(out_error, result.error);
        return false;
    }
    if (peer->subscription_count == peer->subscription_capacity) {
        size_t capacity = peer->subscription_capacity == 0 ? 4 : peer->subscription_capacity * 2;
        void *resized = realloc(peer->subscriptions, capacity * sizeof(*peer->subscriptions));
        if (resized == NULL) {
            dittoffi_sync_subscription_cancel(result.success);
            dittoffi_sync_subscription_free(result.success);
            set_literal_error(out_error, "allocating Ditto subscription list failed");
            return false;
        }
        peer->subscriptions = resized;
        peer->subscription_capacity = capacity;
    }
    peer->subscriptions[peer->subscription_count++] = result.success;
    return true;
}

bool autonomy_sim_ditto_peer_set_transport(
    autonomy_sim_ditto_peer_t *peer,
    const uint8_t *config,
    size_t config_length,
    char **out_error) {
    slice_ref_uint8_t config_slice = {config, config_length};
    dittoffi_result_void_t result =
        dittoffi_ditto_try_set_transport_config(peer->ditto, config_slice, true);
    if (result.error != NULL) {
        take_ditto_error(out_error, result.error);
        return false;
    }
    return true;
}

bool autonomy_sim_ditto_peer_start(
    autonomy_sim_ditto_peer_t *peer,
    char **out_error) {
    dittoffi_result_void_t result = dittoffi_ditto_try_start_sync(peer->ditto);
    if (result.error != NULL) {
        take_ditto_error(out_error, result.error);
        return false;
    }
    return true;
}

bool autonomy_sim_ditto_peer_exec(
    autonomy_sim_ditto_peer_t *peer,
    const char *statement,
    const uint8_t *args,
    size_t args_length,
    char **out_error) {
    slice_ref_uint8_t args_slice = {args, args_length};
    dittoffi_result_dittoffi_query_result_ptr_t result =
        dittoffi_try_exec_statement(peer->ditto, statement, args_slice);
    if (result.error != NULL) {
        take_ditto_error(out_error, result.error);
        return false;
    }
    dittoffi_query_result_free(result.success);
    return true;
}

bool autonomy_sim_ditto_peer_query_json(
    autonomy_sim_ditto_peer_t *peer,
    const char *statement,
    const uint8_t *args,
    size_t args_length,
    char **out_json,
    char **out_error) {
    slice_ref_uint8_t args_slice = {args, args_length};
    dittoffi_result_dittoffi_query_result_ptr_t result =
        dittoffi_try_exec_statement(peer->ditto, statement, args_slice);
    if (result.error != NULL) {
        take_ditto_error(out_error, result.error);
        return false;
    }

    size_t item_count = dittoffi_query_result_item_count(result.success);
    char **items = calloc(item_count == 0 ? 1 : item_count, sizeof(*items));
    if (items == NULL) {
        dittoffi_query_result_free(result.success);
        set_literal_error(out_error, "allocating Ditto query result failed");
        return false;
    }
    size_t total = 3;
    for (size_t index = 0; index < item_count; ++index) {
        dittoffi_query_result_item_t *item =
            dittoffi_query_result_item_at(result.success, index);
        items[index] = dittoffi_query_result_item_json(item);
        dittoffi_query_result_item_free(item);
        if (items[index] == NULL) {
            for (size_t prior = 0; prior < index; ++prior) {
                ditto_c_string_free(items[prior]);
            }
            free(items);
            dittoffi_query_result_free(result.success);
            set_literal_error(out_error, "serializing Ditto query item failed");
            return false;
        }
        total += strlen(items[index]) + 1;
    }

    char *json = malloc(total);
    if (json == NULL) {
        for (size_t index = 0; index < item_count; ++index) {
            ditto_c_string_free(items[index]);
        }
        free(items);
        dittoffi_query_result_free(result.success);
        set_literal_error(out_error, "allocating Ditto JSON result failed");
        return false;
    }
    char *cursor = json;
    *cursor++ = '[';
    for (size_t index = 0; index < item_count; ++index) {
        if (index != 0) {
            *cursor++ = ',';
        }
        size_t length = strlen(items[index]);
        memcpy(cursor, items[index], length);
        cursor += length;
        ditto_c_string_free(items[index]);
    }
    *cursor++ = ']';
    *cursor = '\0';
    free(items);
    dittoffi_query_result_free(result.success);
    *out_json = json;
    return true;
}

void autonomy_sim_ditto_peer_free(autonomy_sim_ditto_peer_t *peer) {
    if (peer == NULL) {
        return;
    }
    dittoffi_ditto_stop_sync(peer->ditto);
    for (size_t index = 0; index < peer->subscription_count; ++index) {
        dittoffi_sync_subscription_cancel(peer->subscriptions[index]);
        dittoffi_sync_subscription_free(peer->subscriptions[index]);
    }
    free(peer->subscriptions);
    ditto_free(peer->ditto);
    free(peer);
}

void autonomy_sim_ditto_string_free(char *value) {
    free(value);
}
