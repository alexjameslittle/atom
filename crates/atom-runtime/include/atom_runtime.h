#ifndef ATOM_RUNTIME_H_
#define ATOM_RUNTIME_H_

#include <stddef.h>
#include <stdint.h>

typedef struct AtomSlice {
  const uint8_t *ptr;
  size_t len;
} AtomSlice;

typedef struct AtomOwnedBuffer {
  uint8_t *ptr;
  size_t len;
  size_t cap;
} AtomOwnedBuffer;

typedef enum AtomLifecycleEventCode {
  ATOM_LIFECYCLE_FOREGROUND = 1,
  ATOM_LIFECYCLE_BACKGROUND = 2,
  ATOM_LIFECYCLE_SUSPEND = 3,
  ATOM_LIFECYCLE_RESUME = 4,
  ATOM_LIFECYCLE_TERMINATE = 5,
} AtomLifecycleEventCode;

int32_t atom_app_init(
    AtomSlice config_flatbuffer,
    AtomOwnedBuffer *out_error_flatbuffer);

int32_t atom_app_handle_lifecycle(
    uint32_t event,
    AtomOwnedBuffer *out_error_flatbuffer);

void atom_app_shutdown(void);
void atom_buffer_free(AtomOwnedBuffer buffer);

#endif  // ATOM_RUNTIME_H_
