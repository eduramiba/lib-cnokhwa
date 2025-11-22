import ctypes
import time
import numpy as np
import cv2
import time
from sys import platform

# assuming we are on x64 Python:
if platform == "linux" or platform == "linux2":
    libpath = 'target/release/libcnokhwa.so'
elif platform == "darwin":
    libpath = 'target/release/libcnokhwa.dylib'
elif platform == "win32":
    libpath = 'target/x86_64-win7-windows-msvc/release/cnokhwa.dll'

lib = ctypes.CDLL(libpath)

lib.cnokhwa_initialize.argtypes = []
lib.cnokhwa_initialize.restype = ctypes.c_int32

lib.cnokhwa_ask_videocapture_auth.argtypes = []

lib.cnokhwa_has_videocapture_auth.argtypes = []
lib.cnokhwa_has_videocapture_auth.restype = ctypes.c_int32

lib.cnokhwa_devices_count.argtypes = []
lib.cnokhwa_devices_count.restype = ctypes.c_int32

lib.cnokhwa_device_name.argtypes = [ctypes.c_int32, ctypes.POINTER(ctypes.c_char), ctypes.c_size_t]
lib.cnokhwa_device_name.restype = ctypes.c_size_t

lib.cnokhwa_device_unique_id.argtypes = [ctypes.c_int32, ctypes.POINTER(ctypes.c_char), ctypes.c_size_t]
lib.cnokhwa_device_unique_id.restype = ctypes.c_size_t

lib.cnokhwa_device_model_id.argtypes = [ctypes.c_int32, ctypes.POINTER(ctypes.c_char), ctypes.c_size_t]
lib.cnokhwa_device_model_id.restype = ctypes.c_size_t

lib.cnokhwa_device_format_width.argtypes = [ctypes.c_int32, ctypes.c_int32]
lib.cnokhwa_device_format_width.restype = ctypes.c_int32

lib.cnokhwa_device_format_height.argtypes = [ctypes.c_int32, ctypes.c_int32]
lib.cnokhwa_device_format_height.restype = ctypes.c_int32

lib.cnokhwa_device_format_frame_rate.argtypes = [ctypes.c_int32, ctypes.c_int32]
lib.cnokhwa_device_format_frame_rate.restype = ctypes.c_int32

lib.cnokhwa_device_format_type.argtypes = [ctypes.c_int32, ctypes.c_int32, ctypes.POINTER(ctypes.c_char), ctypes.c_size_t]
lib.cnokhwa_device_format_type.restype = ctypes.c_size_t

lib.cnokhwa_has_first_frame.argtypes = [ctypes.c_int32]
lib.cnokhwa_has_first_frame.restype = ctypes.c_int32

lib.cnokhwa_frame_width.argtypes = [ctypes.c_int32]
lib.cnokhwa_frame_width.restype = ctypes.c_int32

lib.cnokhwa_frame_height.argtypes = [ctypes.c_int32]
lib.cnokhwa_frame_height.restype = ctypes.c_int32

lib.cnokhwa_frame_bytes_per_row.argtypes = [ctypes.c_int32]
lib.cnokhwa_frame_bytes_per_row.restype = ctypes.c_int32

lib.cnokhwa_grab_frame.argtypes = [ctypes.c_int32, ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t]
lib.cnokhwa_grab_frame.restype = ctypes.c_int32

OK = 0
RESULT_YES = OK
RESULT_NO = -256

def get_string_from_function(func, *args, buffer_size=256):
    buf = (ctypes.c_char * buffer_size)()
    func(*args, buf, buffer_size)
    return buf.value.decode('utf-8')

if lib.cnokhwa_initialize() != OK:
    print("Initialization failed")
    exit(1)

if lib.cnokhwa_has_videocapture_auth() != 0:
    print('Asking videocapture auth...')
    lib.cnokhwa_ask_videocapture_auth()
else:
    print('Already has videocapture auth')

device_count = lib.cnokhwa_devices_count()
print(f"Number of devices: {device_count}")

if device_count < 1:
    exit(0)

max_width = 0
max_height = 0

for device_index in range(device_count):
    name = get_string_from_function(lib.cnokhwa_device_name, device_index)
    unique_id = get_string_from_function(lib.cnokhwa_device_unique_id, device_index)
    model_id = get_string_from_function(lib.cnokhwa_device_model_id, device_index)

    print(f"Device {device_index + 1}:")
    print(f"  Name: {name}")
    print(f"  Unique ID: {unique_id}")
    print(f"  Model ID: {model_id}")

    format_count = lib.cnokhwa_device_formats_count(device_index)
    print(f"  Number of formats: {format_count}")

    for format_index in range(format_count):
        width = lib.cnokhwa_device_format_width(device_index, format_index)
        height = lib.cnokhwa_device_format_height(device_index, format_index)
        fps = lib.cnokhwa_device_format_frame_rate(device_index, format_index)
        format_type = get_string_from_function(
            lib.cnokhwa_device_format_type, device_index, format_index
        )

        if device_index == 0 and width > max_width:
            max_width = width
            max_height = height

        print(f"    Format {format_index + 1}: {width}x{height} {fps}fps, Type: {format_type}")

# Staring capture
device_index = 0
width = max_width
height = max_height

# Start capture
result = lib.cnokhwa_start_capture(device_index, width, height)
if result != OK:
    print(f"Error starting capture: {result}")
    exit(1)

try:
    time.sleep(1)
    # Wait for a new frame
    print("Grabbing a frame...")
    while True:
        has_frame = lib.cnokhwa_has_first_frame(device_index)
        if has_frame == RESULT_NO:
            print('Still no frame available')
            time.sleep(0.01)
            continue
        elif has_frame == RESULT_YES:
            print("First frame available!")
            break
        else:
            print(f"Error checking for first frame: {has_frame}")
            break

    # Get frame dimensions
    frame_width = lib.cnokhwa_frame_width(device_index)
    frame_height = lib.cnokhwa_frame_height(device_index)
    bytes_per_row = lib.cnokhwa_frame_bytes_per_row(device_index)
    buffer_size = frame_width * frame_height * 3  # RGB format

    print(frame_width, frame_height, bytes_per_row)

    # Allocate buffer
    buffer = (ctypes.c_uint8 * buffer_size)()

    # Grab the frame
    start = time.time()
    result = lib.cnokhwa_grab_frame(device_index, buffer, buffer_size)
    if result != OK:
        print(f"Error grabbing frame: {result}")
        exit(1)

    print("Grab time: " + str(time.time() - start))

    # Convert to NumPy array
    frame_array = np.ctypeslib.as_array(buffer)

    # Handle potential padding (stride)
    if bytes_per_row > frame_width * 3:
        # Reshape to (frame_height, bytes_per_row)
        frame_array = frame_array.reshape((frame_height, bytes_per_row))

        # Extract the actual image data
        image_data = frame_array[:, :frame_width * 3]

        # Reshape to (frame_height, frame_width, 3)
        frame_array = image_data.reshape((frame_height, frame_width, 3))
    else:
        # No padding, reshape directly
        frame_array = frame_array.reshape((frame_height, frame_width, 3))

    # Ensure data type is uint8
    frame_array = frame_array.astype(np.uint8)

    # Convert RGBA to BGR
    frame_bgr = cv2.cvtColor(frame_array, cv2.COLOR_RGBA2BGR)

    print(f"frame_array.shape: {frame_array.shape}")
    print(f"frame_array.dtype: {frame_array.dtype}")

    print(f"frame_bgr.shape: {frame_bgr.shape}")
    print(f"frame_bgr.dtype: {frame_bgr.dtype}")

    filename = 'frame.jpg'
    print('Writing to ' + filename)

    if not cv2.imwrite(filename, frame_bgr):
        print('Error writing image')
    else:
        print('Image written successfully')

finally:
    # Stop capture
    result = lib.cnokhwa_stop_capture(device_index)
    if result != OK:
        print(f"Error stopping capture: {result}")