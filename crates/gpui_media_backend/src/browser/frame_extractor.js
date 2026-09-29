// A paused, private video element supplies decoded frames without playing audio.
export function extractFrame(video, seconds, timeout) {
    return new Promise((resolve, reject) => {
        let started = false;
        let finished = false;
        const events = ["loadedmetadata", "loadeddata", "seeked", "canplay", "error"];
        const cleanup = () => {
            clearTimeout(timer);
            for (const event of events) video.removeEventListener(event, check);
        };
        const fail = (error) => {
            if (finished) return;
            finished = true;
            cleanup();
            reject(error);
        };
        const check = () => {
            if (finished) return;
            try {
                if (video.error) throw new Error(video.error.message || `Media error ${video.error.code}`);
                if (video.readyState < 1) return;
                if (!started) {
                    started = true;
                    if (Number.isFinite(video.duration) && seconds >= video.duration && seconds > 0) {
                        throw new RangeError("Frame position must be before the end of the video");
                    }
                    // A fresh initial-frame request needs no seek or byte-range support.
                    if (video.currentTime !== seconds) {
                        video.currentTime = seconds;
                        return;
                    }
                }
                if (video.seeking || video.readyState < 2) return;
                if (Math.abs(video.currentTime - seconds) > 0.05) {
                    throw new RangeError("Video source cannot seek to the requested position");
                }
                const frame = new VideoFrame(video);
                finished = true;
                cleanup();
                resolve(frame);
            } catch (error) { fail(error); }
        };
        const timer = setTimeout(() => fail(new DOMException("Frame extraction timed out", "TimeoutError")), timeout);
        for (const event of events) video.addEventListener(event, check);
        check();
    });
}
