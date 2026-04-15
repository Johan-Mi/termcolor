/*!
This crate provides a cross platform abstraction for writing colored text to a
terminal. Colors are written using ANSI escape sequences. Much of this API was
motivated by use inside command line applications, where colors or styles can be
configured by the end user and/or the environment.

This crate also provides platform independent support for writing colored text
to an in memory buffer.

This crate also provides support for writing hyperlinks.

# Organization

The `WriteColor` trait extends the `io::Write` trait with methods for setting
colors or resetting them.

`StandardStream` and `StandardStreamLock` both satisfy `WriteColor` and are
analogous to `std::io::Stdout` and `std::io::StdoutLock`, or `std::io::Stderr`
and `std::io::StderrLock`.

`Buffer` is an in memory buffer that supports colored text. In a parallel
program, each thread might write to its own buffer. A buffer can be printed to
using a `BufferWriter`. This design prevents interleaving of buffer output.

`Ansi` and `NoColor` both satisfy `WriteColor` for arbitrary implementors of
`io::Write`. These types are useful when you know exactly what you need.

# Example: using `StandardStream`

The `StandardStream` type in this crate works similarly to `std::io::Stdout`,
except it is augmented with methods for coloring by the `WriteColor` trait.
For example, to write some green text:

```rust,no_run
# fn test() -> Result<(), Box<dyn std::error::Error>> {
use std::io::Write;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

let mut stdout = StandardStream::stdout(ColorChoice::Always);
stdout.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
writeln!(&mut stdout, "green text!")?;
# Ok(()) }
```

Note that any text written to the terminal now will be colored
green when using ANSI escape sequences, even if it is written via
stderr, and even if stderr had previously been set to `Color::Red`.
Users will need to manage any color changes themselves by calling
[`WriteColor::set_color`](trait.WriteColor.html#tymethod.set_color), and this
may include calling [`WriteColor::reset`](trait.WriteColor.html#tymethod.reset)
before the program exits to a shell.

# Example: using `BufferWriter`

A `BufferWriter` can create buffers and write buffers to stdout or stderr. It
does *not* implement `io::Write` or `WriteColor` itself. Instead, `Buffer`
implements `io::Write` and `io::WriteColor`.

This example shows how to print some green text to stderr.

```rust,no_run
# fn test() -> Result<(), Box<dyn std::error::Error>> {
use std::io::Write;
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

let mut bufwtr = BufferWriter::stderr(ColorChoice::Always);
let mut buffer = bufwtr.buffer();
buffer.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
writeln!(&mut buffer, "green text!")?;
bufwtr.print(&buffer)?;
# Ok(()) }
```

# Detecting presence of a terminal

In many scenarios when using color, one often wants to enable colors
automatically when writing to a terminal and disable colors automatically when
writing to anything else. The typical way to achieve this is to use the standard
library's [`IsTerminal`](https://doc.rust-lang.org/std/io/trait.IsTerminal.html)
trait.

For example, in a command line application that exposes a `--color` flag,
your logic for how to enable colors might look like this:

```ignore
use std::io::IsTerminal;

use termcolor::{ColorChoice, StandardStream};

let preference = argv.get_flag("color").unwrap_or("auto");
let mut choice = preference.parse::<ColorChoice>()?;
if choice == ColorChoice::Auto && !std::io::stdin().is_terminal() {
    choice = ColorChoice::Never;
}
let stdout = StandardStream::stdout(choice);
// ... write to stdout
```

Currently, `termcolor` does not provide anything to do this for you.
*/

#![deny(missing_debug_implementations, missing_docs)]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
const _: () = ();

use std::env;
use std::error;
use std::fmt;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

/// This trait describes the behavior of writers that support colored output.
pub trait WriteColor: io::Write {
    /// Returns true if and only if the underlying writer supports colors.
    fn supports_color(&self) -> bool;

    /// Set the color settings of the writer.
    ///
    /// Subsequent writes to this writer will use these settings until either
    /// `reset` is called or new color settings are set.
    ///
    /// If there was a problem setting the color settings, then an error is
    /// returned.
    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()>;

    /// Reset the current color settings to their original settings.
    ///
    /// If there was a problem resetting the color settings, then an error is
    /// returned.
    ///
    /// Note that this does not reset hyperlinks. Those need to be
    /// reset on their own, e.g., by calling `set_hyperlink` with
    /// [`HyperlinkSpec::none`].
    fn reset(&mut self) -> io::Result<()>;

    /// Returns true if and only if the underlying writer must synchronously
    /// interact with an end user's device in order to control colors. By
    /// default, this always returns `false`.
    ///
    /// This is useful for writing generic code (such as a buffered writer)
    /// that can perform certain optimizations when the underlying writer
    /// doesn't rely on synchronous APIs. For example, ANSI escape sequences
    /// can be passed through to the end user's device as is.
    fn is_synchronous(&self) -> bool {
        false
    }

    /// Set the current hyperlink of the writer.
    ///
    /// The typical way to use this is to first call it with a
    /// [`HyperlinkSpec::open`] to write the actual URI to a tty that supports
    /// [OSC-8]. At this point, the caller can now write the label for the
    /// hyperlink. This may include coloring or other styles. Once the caller
    /// has finished writing the label, one should call this method again with
    /// [`HyperlinkSpec::close`].
    ///
    /// If there was a problem setting the hyperlink, then an error is
    /// returned.
    ///
    /// This defaults to doing nothing.
    ///
    /// [OSC8]: https://github.com/Alhadis/OSC8-Adoption/
    fn set_hyperlink(&mut self, _link: &HyperlinkSpec) -> io::Result<()> {
        Ok(())
    }

    /// Returns true if and only if the underlying writer supports hyperlinks.
    ///
    /// This can be used to avoid generating hyperlink URIs unnecessarily.
    ///
    /// This defaults to `false`.
    fn supports_hyperlinks(&self) -> bool {
        false
    }
}

impl<T: ?Sized + WriteColor> WriteColor for &mut T {
    fn supports_color(&self) -> bool {
        (**self).supports_color()
    }
    fn supports_hyperlinks(&self) -> bool {
        (**self).supports_hyperlinks()
    }
    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        (**self).set_color(spec)
    }
    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        (**self).set_hyperlink(link)
    }
    fn reset(&mut self) -> io::Result<()> {
        (**self).reset()
    }
    fn is_synchronous(&self) -> bool {
        (**self).is_synchronous()
    }
}

impl<T: ?Sized + WriteColor> WriteColor for Box<T> {
    fn supports_color(&self) -> bool {
        (**self).supports_color()
    }
    fn supports_hyperlinks(&self) -> bool {
        (**self).supports_hyperlinks()
    }
    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        (**self).set_color(spec)
    }
    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        (**self).set_hyperlink(link)
    }
    fn reset(&mut self) -> io::Result<()> {
        (**self).reset()
    }
    fn is_synchronous(&self) -> bool {
        (**self).is_synchronous()
    }
}

/// `ColorChoice` represents the color preferences of an end user.
///
/// The `Default` implementation for this type will select `Auto`, which tries
/// to do the right thing based on the current environment.
///
/// The `FromStr` implementation for this type converts a lowercase kebab-case
/// string of the variant name to the corresponding variant. Any other string
/// results in an error.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorChoice {
    /// Try very hard to emit colors.
    Always,
    /// Try to use colors, but don't force the issue. If `TERM=dumb`, or if
    /// `NO_COLOR` is defined, for example, then don't use colors.
    #[default]
    Auto,
    /// Never emit colors.
    Never,
}

impl FromStr for ColorChoice {
    type Err = ColorChoiceParseError;

    fn from_str(s: &str) -> Result<Self, ColorChoiceParseError> {
        match s.to_lowercase().as_str() {
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            "auto" => Ok(Self::Auto),
            unknown => Err(ColorChoiceParseError {
                unknown_choice: unknown.to_string(),
            }),
        }
    }
}

impl ColorChoice {
    /// Returns true if we should attempt to write colored output.
    fn should_attempt_color(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => Self::env_allows_color(),
        }
    }

    fn env_allows_color() -> bool {
        env::var_os("NO_COLOR").is_none_or(|it| it.is_empty() || it == "0")
            && (env::var_os("CLICOLOR_FORCE")
                .is_some_and(|it| !(it.is_empty() || it == "0"))
                || (env::var_os("CLICOLOR").is_none_or(|it| it != "0")
                    // If TERM isn't set, then we are in a weird environment that
                    // probably doesn't support colors, or Windows.
                    && env::var_os("TERM")
                        .map_or(cfg!(windows), |it| it != "dumb")))
    }
}

/// An error that occurs when parsing a `ColorChoice` fails.
#[derive(Clone, Debug)]
pub struct ColorChoiceParseError {
    unknown_choice: String,
}

impl std::error::Error for ColorChoiceParseError {}

impl fmt::Display for ColorChoiceParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "unrecognized color choice '{}': valid choices are: \
             always, always-ansi, never, auto",
            self.unknown_choice,
        )
    }
}

// `std::io` implements `Stdout` and `Stderr` (and their `Lock` variants) as
// separate types, which makes it difficult to abstract over them. We use
// some simple internal enum types to work around this.

#[derive(Debug)]
enum IoStandardStream {
    Stdout(io::Stdout),
    Stderr(io::Stderr),
    StdoutBuffered(io::BufWriter<io::Stdout>),
    StderrBuffered(io::BufWriter<io::Stderr>),
}

impl IoStandardStream {
    fn lock(&self) -> IoStandardStreamLock {
        match self {
            Self::Stdout(s) => IoStandardStreamLock::StdoutLock(s.lock()),
            Self::Stderr(s) => IoStandardStreamLock::StderrLock(s.lock()),
            Self::StdoutBuffered(_) | Self::StderrBuffered(_) => {
                // We don't permit this case to ever occur in the public API,
                // so it's OK to panic.
                panic!("cannot lock a buffered standard stream")
            }
        }
    }
}

impl io::Write for IoStandardStream {
    #[inline]
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Self::Stdout(s) => s.write(b),
            Self::Stderr(s) => s.write(b),
            Self::StdoutBuffered(s) => s.write(b),
            Self::StderrBuffered(s) => s.write(b),
        }
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Stdout(s) => s.flush(),
            Self::Stderr(s) => s.flush(),
            Self::StdoutBuffered(s) => s.flush(),
            Self::StderrBuffered(s) => s.flush(),
        }
    }
}

// Same rigmarole for the locked variants of the standard streams.

#[derive(Debug)]
enum IoStandardStreamLock {
    StdoutLock(io::StdoutLock<'static>),
    StderrLock(io::StderrLock<'static>),
}

impl io::Write for IoStandardStreamLock {
    #[inline]
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Self::StdoutLock(s) => s.write(b),
            Self::StderrLock(s) => s.write(b),
        }
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::StdoutLock(s) => s.flush(),
            Self::StderrLock(s) => s.flush(),
        }
    }
}

/// Satisfies `io::Write` and `WriteColor`, and supports optional coloring
/// to either of the standard output streams, stdout and stderr.
#[derive(Debug)]
pub struct StandardStream {
    wtr: WriterInner<IoStandardStream>,
}

/// `StandardStreamLock` is a locked reference to a `StandardStream`.
///
/// This implements the `io::Write` and `WriteColor` traits, and is constructed
/// via the `Write::lock` method.
#[derive(Debug)]
pub struct StandardStreamLock {
    wtr: WriterInnerLock<IoStandardStreamLock>,
}

/// Like `StandardStream`, but does buffered writing.
#[derive(Debug)]
pub struct BufferedStandardStream {
    wtr: WriterInner<IoStandardStream>,
}

/// `WriterInner` is a (limited) generic representation of a writer. It is
/// limited because W should only ever be stdout/stderr on Windows.
#[derive(Debug)]
enum WriterInner<W> {
    NoColor(NoColor<W>),
    Ansi(Ansi<W>),
}

/// `WriterInnerLock` is a (limited) generic representation of a writer. It is
/// limited because W should only ever be stdout/stderr on Windows.
#[derive(Debug)]
enum WriterInnerLock<W> {
    NoColor(NoColor<W>),
    Ansi(Ansi<W>),
}

impl StandardStream {
    /// Create a new `StandardStream` with the given color preferences that
    /// writes to standard output.
    ///
    /// The specific color/style settings can be configured when writing via
    /// the `WriteColor` trait.
    pub fn stdout(choice: ColorChoice) -> Self {
        let wtr = WriterInner::create(
            IoStandardStream::Stdout(io::stdout()),
            choice,
        );
        Self { wtr }
    }

    /// Create a new `StandardStream` with the given color preferences that
    /// writes to standard error.
    ///
    /// The specific color/style settings can be configured when writing via
    /// the `WriteColor` trait.
    pub fn stderr(choice: ColorChoice) -> Self {
        let wtr = WriterInner::create(
            IoStandardStream::Stderr(io::stderr()),
            choice,
        );
        Self { wtr }
    }

    /// Lock the underlying writer.
    ///
    /// The lock guard returned also satisfies `io::Write` and
    /// `WriteColor`.
    ///
    /// This method is **not reentrant**. It may panic if `lock` is called
    /// while a `StandardStreamLock` is still alive.
    pub fn lock(&self) -> StandardStreamLock {
        StandardStreamLock::from_stream(self)
    }
}

impl StandardStreamLock {
    fn from_stream(stream: &StandardStream) -> Self {
        let locked = match &stream.wtr {
            WriterInner::NoColor(w) => {
                WriterInnerLock::NoColor(NoColor(w.0.lock()))
            }
            WriterInner::Ansi(w) => WriterInnerLock::Ansi(Ansi(w.0.lock())),
        };
        Self { wtr: locked }
    }
}

impl BufferedStandardStream {
    /// Create a new `BufferedStandardStream` with the given color preferences
    /// that writes to standard output via a buffered writer.
    ///
    /// The specific color/style settings can be configured when writing via
    /// the `WriteColor` trait.
    pub fn stdout(choice: ColorChoice) -> Self {
        let wtr = WriterInner::create(
            IoStandardStream::StdoutBuffered(io::BufWriter::new(io::stdout())),
            choice,
        );
        Self { wtr }
    }

    /// Create a new `BufferedStandardStream` with the given color preferences
    /// that writes to standard error via a buffered writer.
    ///
    /// The specific color/style settings can be configured when writing via
    /// the `WriteColor` trait.
    pub fn stderr(choice: ColorChoice) -> Self {
        let wtr = WriterInner::create(
            IoStandardStream::StderrBuffered(io::BufWriter::new(io::stderr())),
            choice,
        );
        Self { wtr }
    }
}

impl WriterInner<IoStandardStream> {
    /// Create a new inner writer for a standard stream with the given color
    /// preferences.
    fn create(stream: IoStandardStream, choice: ColorChoice) -> Self {
        if choice.should_attempt_color() {
            Self::Ansi(Ansi(stream))
        } else {
            Self::NoColor(NoColor(stream))
        }
    }
}

impl io::Write for StandardStream {
    #[inline]
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.wtr.write(b)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.wtr.flush()
    }
}

impl WriteColor for StandardStream {
    #[inline]
    fn supports_color(&self) -> bool {
        self.wtr.supports_color()
    }

    #[inline]
    fn supports_hyperlinks(&self) -> bool {
        self.wtr.supports_hyperlinks()
    }

    #[inline]
    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        self.wtr.set_color(spec)
    }

    #[inline]
    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        self.wtr.set_hyperlink(link)
    }

    #[inline]
    fn reset(&mut self) -> io::Result<()> {
        self.wtr.reset()
    }

    #[inline]
    fn is_synchronous(&self) -> bool {
        self.wtr.is_synchronous()
    }
}

impl io::Write for StandardStreamLock {
    #[inline]
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.wtr.write(b)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.wtr.flush()
    }
}

impl WriteColor for StandardStreamLock {
    #[inline]
    fn supports_color(&self) -> bool {
        self.wtr.supports_color()
    }

    #[inline]
    fn supports_hyperlinks(&self) -> bool {
        self.wtr.supports_hyperlinks()
    }

    #[inline]
    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        self.wtr.set_color(spec)
    }

    #[inline]
    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        self.wtr.set_hyperlink(link)
    }

    #[inline]
    fn reset(&mut self) -> io::Result<()> {
        self.wtr.reset()
    }

    #[inline]
    fn is_synchronous(&self) -> bool {
        self.wtr.is_synchronous()
    }
}

impl io::Write for BufferedStandardStream {
    #[inline]
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.wtr.write(b)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.wtr.flush()
    }
}

impl WriteColor for BufferedStandardStream {
    #[inline]
    fn supports_color(&self) -> bool {
        self.wtr.supports_color()
    }

    #[inline]
    fn supports_hyperlinks(&self) -> bool {
        self.wtr.supports_hyperlinks()
    }

    #[inline]
    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        if self.is_synchronous() {
            self.wtr.flush()?;
        }
        self.wtr.set_color(spec)
    }

    #[inline]
    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        if self.is_synchronous() {
            self.wtr.flush()?;
        }
        self.wtr.set_hyperlink(link)
    }

    #[inline]
    fn reset(&mut self) -> io::Result<()> {
        self.wtr.reset()
    }

    #[inline]
    fn is_synchronous(&self) -> bool {
        self.wtr.is_synchronous()
    }
}

impl<W: io::Write> io::Write for WriterInner<W> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::NoColor(wtr) => wtr.write(buf),
            Self::Ansi(wtr) => wtr.write(buf),
        }
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::NoColor(wtr) => wtr.flush(),
            Self::Ansi(wtr) => wtr.flush(),
        }
    }
}

impl<W: io::Write> WriteColor for WriterInner<W> {
    fn supports_color(&self) -> bool {
        match self {
            Self::NoColor(_) => false,
            Self::Ansi(_) => true,
        }
    }

    fn supports_hyperlinks(&self) -> bool {
        match self {
            Self::NoColor(_) => false,
            Self::Ansi(_) => true,
        }
    }

    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        match self {
            Self::NoColor(wtr) => wtr.set_color(spec),
            Self::Ansi(wtr) => wtr.set_color(spec),
        }
    }

    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        match self {
            Self::NoColor(wtr) => wtr.set_hyperlink(link),
            Self::Ansi(wtr) => wtr.set_hyperlink(link),
        }
    }

    fn reset(&mut self) -> io::Result<()> {
        match self {
            Self::NoColor(wtr) => wtr.reset(),
            Self::Ansi(wtr) => wtr.reset(),
        }
    }

    fn is_synchronous(&self) -> bool {
        false
    }
}

impl<W: io::Write> io::Write for WriterInnerLock<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::NoColor(wtr) => wtr.write(buf),
            Self::Ansi(wtr) => wtr.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::NoColor(wtr) => wtr.flush(),
            Self::Ansi(wtr) => wtr.flush(),
        }
    }
}

impl<W: io::Write> WriteColor for WriterInnerLock<W> {
    fn supports_color(&self) -> bool {
        match self {
            Self::NoColor(_) => false,
            Self::Ansi(_) => true,
        }
    }

    fn supports_hyperlinks(&self) -> bool {
        match self {
            Self::NoColor(_) => false,
            Self::Ansi(_) => true,
        }
    }

    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        match self {
            Self::NoColor(wtr) => wtr.set_color(spec),
            Self::Ansi(wtr) => wtr.set_color(spec),
        }
    }

    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        match self {
            Self::NoColor(wtr) => wtr.set_hyperlink(link),
            Self::Ansi(wtr) => wtr.set_hyperlink(link),
        }
    }

    fn reset(&mut self) -> io::Result<()> {
        match self {
            Self::NoColor(wtr) => wtr.reset(),
            Self::Ansi(wtr) => wtr.reset(),
        }
    }

    fn is_synchronous(&self) -> bool {
        false
    }
}

/// Writes colored buffers to stdout or stderr.
///
/// Writable buffers can be obtained by calling `buffer` on a `BufferWriter`.
///
/// This writer works with terminals that support ANSI escape sequences.
///
/// It is intended for a `BufferWriter` to be put in an `Arc` and written to
/// from multiple threads simultaneously.
#[derive(Debug)]
pub struct BufferWriter {
    stream: IoStandardStream,
    printed: AtomicBool,
    separator: Option<Vec<u8>>,
    color_choice: ColorChoice,
}

impl BufferWriter {
    /// Create a new `BufferWriter` that writes to a standard stream with the
    /// given color preferences.
    ///
    /// The specific color/style settings can be configured when writing to
    /// the buffers themselves.
    fn create(stream: IoStandardStream, choice: ColorChoice) -> Self {
        Self {
            stream,
            printed: AtomicBool::new(false),
            separator: None,
            color_choice: choice,
        }
    }

    /// Create a new `BufferWriter` that writes to stdout with the given
    /// color preferences.
    ///
    /// The specific color/style settings can be configured when writing to
    /// the buffers themselves.
    pub fn stdout(choice: ColorChoice) -> Self {
        Self::create(IoStandardStream::Stdout(io::stdout()), choice)
    }

    /// Create a new `BufferWriter` that writes to stderr with the given
    /// color preferences.
    ///
    /// The specific color/style settings can be configured when writing to
    /// the buffers themselves.
    pub fn stderr(choice: ColorChoice) -> Self {
        Self::create(IoStandardStream::Stderr(io::stderr()), choice)
    }

    /// If set, the separator given is printed between buffers. By default, no
    /// separator is printed.
    ///
    /// The default value is `None`.
    pub fn separator(&mut self, sep: Option<Vec<u8>>) {
        self.separator = sep;
    }

    /// Creates a new `Buffer` with the current color preferences.
    ///
    /// A `Buffer` satisfies both `io::Write` and `WriteColor`. A `Buffer` can
    /// be printed using the `print` method.
    pub fn buffer(&self) -> Buffer {
        Buffer::new(self.color_choice)
    }

    /// Prints the contents of the given buffer.
    ///
    /// It is safe to call this from multiple threads simultaneously. In
    /// particular, all buffers are written atomically. No interleaving will
    /// occur.
    pub fn print(&self, buf: &Buffer) -> io::Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let mut stream = self.stream.lock();
        if let Some(sep) = &self.separator
            && self.printed.load(Ordering::Relaxed)
        {
            stream.write_all(sep)?;
            stream.write_all(b"\n")?;
        }
        match &buf.0 {
            BufferInner::NoColor(b) => stream.write_all(&b.0)?,
            BufferInner::Ansi(b) => stream.write_all(&b.0)?,
        }
        self.printed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// Write colored text to memory.
///
/// `Buffer` is a platform independent abstraction for printing colored text to
/// an in memory buffer. When the buffer is printed using a `BufferWriter`, the
/// color information will be applied to the output device.
///
/// A `Buffer` is typically created by calling the `BufferWriter.buffer` method,
/// which will take color preferences and the environment into account. However,
/// buffers can also be manually created using `no_color` or `ansi`.
#[derive(Clone, Debug)]
pub struct Buffer(BufferInner);

/// `BufferInner` is an enumeration of different buffer types.
#[derive(Clone, Debug)]
enum BufferInner {
    /// No coloring information should be applied. This ignores all coloring
    /// directives.
    NoColor(NoColor<Vec<u8>>),
    /// Apply coloring using ANSI escape sequences embedded into the buffer.
    Ansi(Ansi<Vec<u8>>),
}

impl Buffer {
    /// Create a new buffer with the given color settings.
    fn new(choice: ColorChoice) -> Self {
        if choice.should_attempt_color() {
            Self::ansi()
        } else {
            Self::no_color()
        }
    }

    /// Create a buffer that drops all color information.
    pub const fn no_color() -> Self {
        Self(BufferInner::NoColor(NoColor(Vec::new())))
    }

    /// Create a buffer that uses ANSI escape sequences.
    pub const fn ansi() -> Self {
        Self(BufferInner::Ansi(Ansi(Vec::new())))
    }

    /// Returns true if and only if this buffer is empty.
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the length of this buffer in bytes.
    pub const fn len(&self) -> usize {
        match &self.0 {
            BufferInner::NoColor(b) => b.0.len(),
            BufferInner::Ansi(b) => b.0.len(),
        }
    }

    /// Clears this buffer.
    pub fn clear(&mut self) {
        match &mut self.0 {
            BufferInner::NoColor(b) => b.0.clear(),
            BufferInner::Ansi(b) => b.0.clear(),
        }
    }

    /// Consume this buffer and return the underlying raw data.
    pub fn into_inner(self) -> Vec<u8> {
        match self.0 {
            BufferInner::NoColor(b) => b.0,
            BufferInner::Ansi(b) => b.0,
        }
    }

    /// Return the underlying data of the buffer.
    pub fn as_slice(&self) -> &[u8] {
        match &self.0 {
            BufferInner::NoColor(b) => &b.0,
            BufferInner::Ansi(b) => &b.0,
        }
    }

    /// Return the underlying data of the buffer as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        match &mut self.0 {
            BufferInner::NoColor(b) => &mut b.0,
            BufferInner::Ansi(b) => &mut b.0,
        }
    }
}

impl io::Write for Buffer {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.0 {
            BufferInner::NoColor(w) => w.write(buf),
            BufferInner::Ansi(w) => w.write(buf),
        }
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        match &mut self.0 {
            BufferInner::NoColor(w) => w.flush(),
            BufferInner::Ansi(w) => w.flush(),
        }
    }
}

impl WriteColor for Buffer {
    #[inline]
    fn supports_color(&self) -> bool {
        match self.0 {
            BufferInner::NoColor(_) => false,
            BufferInner::Ansi(_) => true,
        }
    }

    #[inline]
    fn supports_hyperlinks(&self) -> bool {
        match self.0 {
            BufferInner::NoColor(_) => false,
            BufferInner::Ansi(_) => true,
        }
    }

    #[inline]
    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        match &mut self.0 {
            BufferInner::NoColor(w) => w.set_color(spec),
            BufferInner::Ansi(w) => w.set_color(spec),
        }
    }

    #[inline]
    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        match &mut self.0 {
            BufferInner::NoColor(w) => w.set_hyperlink(link),
            BufferInner::Ansi(w) => w.set_hyperlink(link),
        }
    }

    #[inline]
    fn reset(&mut self) -> io::Result<()> {
        match &mut self.0 {
            BufferInner::NoColor(w) => w.reset(),
            BufferInner::Ansi(w) => w.reset(),
        }
    }

    #[inline]
    fn is_synchronous(&self) -> bool {
        false
    }
}

/// Satisfies `WriteColor` but ignores all color options.
#[derive(Clone, Debug)]
pub struct NoColor<W>(W);

impl<W: Write> NoColor<W> {
    /// Create a new writer that satisfies `WriteColor` but drops all color
    /// information.
    pub const fn new(wtr: W) -> Self {
        Self(wtr)
    }

    /// Consume this `NoColor` value and return the inner writer.
    pub fn into_inner(self) -> W {
        self.0
    }

    /// Return a reference to the inner writer.
    pub const fn get_ref(&self) -> &W {
        &self.0
    }

    /// Return a mutable reference to the inner writer.
    pub const fn get_mut(&mut self) -> &mut W {
        &mut self.0
    }
}

impl<W: io::Write> io::Write for NoColor<W> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<W: io::Write> WriteColor for NoColor<W> {
    #[inline]
    fn supports_color(&self) -> bool {
        false
    }

    #[inline]
    fn supports_hyperlinks(&self) -> bool {
        false
    }

    #[inline]
    fn set_color(&mut self, _: &ColorSpec) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn set_hyperlink(&mut self, _: &HyperlinkSpec) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn reset(&mut self) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn is_synchronous(&self) -> bool {
        false
    }
}

/// Satisfies `WriteColor` using standard ANSI escape sequences.
#[derive(Clone, Debug)]
pub struct Ansi<W>(W);

impl<W: Write> Ansi<W> {
    /// Create a new writer that satisfies `WriteColor` using standard ANSI
    /// escape sequences.
    pub const fn new(wtr: W) -> Self {
        Self(wtr)
    }

    /// Consume this `Ansi` value and return the inner writer.
    pub fn into_inner(self) -> W {
        self.0
    }

    /// Return a reference to the inner writer.
    pub const fn get_ref(&self) -> &W {
        &self.0
    }

    /// Return a mutable reference to the inner writer.
    pub const fn get_mut(&mut self) -> &mut W {
        &mut self.0
    }
}

impl<W: io::Write> io::Write for Ansi<W> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    // Adding this method here is not required because it has a default impl,
    // but it seems to provide a perf improvement in some cases when using
    // a `BufWriter` with lots of writes.
    //
    // See https://github.com/BurntSushi/termcolor/pull/56 for more details
    // and a minimized example.
    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(buf)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<W: io::Write> WriteColor for Ansi<W> {
    #[inline]
    fn supports_color(&self) -> bool {
        true
    }

    #[inline]
    fn supports_hyperlinks(&self) -> bool {
        true
    }

    #[inline]
    fn set_color(&mut self, spec: &ColorSpec) -> io::Result<()> {
        if spec.reset {
            self.reset()?;
        }
        if spec.bold {
            self.write_str("\x1B[1m")?;
        }
        if spec.dimmed {
            self.write_str("\x1B[2m")?;
        }
        if spec.italic {
            self.write_str("\x1B[3m")?;
        }
        if spec.underline {
            self.write_str("\x1B[4m")?;
        }
        if spec.strikethrough {
            self.write_str("\x1B[9m")?;
        }
        if let Some(c) = spec.fg_color {
            self.write_color(true, c, spec.intense)?;
        }
        if let Some(c) = spec.bg_color {
            self.write_color(false, c, spec.intense)?;
        }
        Ok(())
    }

    #[inline]
    fn set_hyperlink(&mut self, link: &HyperlinkSpec) -> io::Result<()> {
        self.write_str("\x1B]8;;")?;
        if let Some(uri) = link.uri() {
            self.write_all(uri)?;
        }
        self.write_str("\x1B\\")
    }

    #[inline]
    fn reset(&mut self) -> io::Result<()> {
        self.write_str("\x1B[0m")
    }

    #[inline]
    fn is_synchronous(&self) -> bool {
        false
    }
}

impl<W: io::Write> Ansi<W> {
    fn write_str(&mut self, s: &str) -> io::Result<()> {
        self.write_all(s.as_bytes())
    }

    fn write_color(
        &mut self,
        fg: bool,
        c: Color,
        intense: bool,
    ) -> io::Result<()> {
        macro_rules! write_intense {
            ($clr:expr) => {
                if fg {
                    self.write_str(concat!("\x1B[38;5;", $clr, "m"))
                } else {
                    self.write_str(concat!("\x1B[48;5;", $clr, "m"))
                }
            };
        }
        macro_rules! write_normal {
            ($clr:expr) => {
                if fg {
                    self.write_str(concat!("\x1B[3", $clr, "m"))
                } else {
                    self.write_str(concat!("\x1B[4", $clr, "m"))
                }
            };
        }
        macro_rules! write_var_ansi_code {
            ($pre:expr, $($code:expr),+) => {{
                // The loop generates at worst a literal of the form
                // '255,255,255m' which is 12-bytes.
                // The largest `pre` expression we currently use is 7 bytes.
                // This gives us the maximum of 19-bytes for our work buffer.
                let pre_len = $pre.len();
                assert!(pre_len <= 7);
                let mut fmt = [0u8; 19];
                fmt[..pre_len].copy_from_slice($pre);
                let mut i = pre_len - 1;
                $(
                    let c1: u8 = ($code / 100) % 10;
                    let c2: u8 = ($code / 10) % 10;
                    let c3: u8 = $code % 10;
                    let mut printed = false;

                    if c1 != 0 {
                        printed = true;
                        i += 1;
                        fmt[i] = b'0' + c1;
                    }
                    if c2 != 0 || printed {
                        i += 1;
                        fmt[i] = b'0' + c2;
                    }
                    // If we received a zero value we must still print a value.
                    i += 1;
                    fmt[i] = b'0' + c3;
                    i += 1;
                    fmt[i] = b';';
                )+

                fmt[i] = b'm';
                self.write_all(&fmt[0..i+1])
            }}
        }
        macro_rules! write_custom {
            ($ansi256:expr) => {
                if fg {
                    write_var_ansi_code!(b"\x1B[38;5;", $ansi256)
                } else {
                    write_var_ansi_code!(b"\x1B[48;5;", $ansi256)
                }
            };

            ($r:expr, $g:expr, $b:expr) => {{
                if fg {
                    write_var_ansi_code!(b"\x1B[38;2;", $r, $g, $b)
                } else {
                    write_var_ansi_code!(b"\x1B[48;2;", $r, $g, $b)
                }
            }};
        }
        if intense {
            match c {
                Color::Black => write_intense!("8"),
                Color::Blue => write_intense!("12"),
                Color::Green => write_intense!("10"),
                Color::Red => write_intense!("9"),
                Color::Cyan => write_intense!("14"),
                Color::Magenta => write_intense!("13"),
                Color::Yellow => write_intense!("11"),
                Color::White => write_intense!("15"),
                Color::Ansi256(c) => write_custom!(c),
                Color::Rgb(r, g, b) => write_custom!(r, g, b),
            }
        } else {
            match c {
                Color::Black => write_normal!("0"),
                Color::Blue => write_normal!("4"),
                Color::Green => write_normal!("2"),
                Color::Red => write_normal!("1"),
                Color::Cyan => write_normal!("6"),
                Color::Magenta => write_normal!("5"),
                Color::Yellow => write_normal!("3"),
                Color::White => write_normal!("7"),
                Color::Ansi256(c) => write_custom!(c),
                Color::Rgb(r, g, b) => write_custom!(r, g, b),
            }
        }
    }
}

impl WriteColor for io::Sink {
    fn supports_color(&self) -> bool {
        false
    }

    fn supports_hyperlinks(&self) -> bool {
        false
    }

    fn set_color(&mut self, _: &ColorSpec) -> io::Result<()> {
        Ok(())
    }

    fn set_hyperlink(&mut self, _: &HyperlinkSpec) -> io::Result<()> {
        Ok(())
    }

    fn reset(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A color specification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColorSpec {
    fg_color: Option<Color>,
    bg_color: Option<Color>,
    bold: bool,
    intense: bool,
    underline: bool,
    dimmed: bool,
    italic: bool,
    reset: bool,
    strikethrough: bool,
}

impl Default for ColorSpec {
    fn default() -> Self {
        Self {
            fg_color: None,
            bg_color: None,
            bold: false,
            intense: false,
            underline: false,
            dimmed: false,
            italic: false,
            reset: true,
            strikethrough: false,
        }
    }
}

impl ColorSpec {
    /// Create a new color specification that has no colors or styles.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the foreground color.
    pub const fn fg(&self) -> Option<&Color> {
        self.fg_color.as_ref()
    }

    /// Set the foreground color.
    pub const fn set_fg(&mut self, color: Option<Color>) -> &mut Self {
        self.fg_color = color;
        self
    }

    /// Get the background color.
    pub const fn bg(&self) -> Option<&Color> {
        self.bg_color.as_ref()
    }

    /// Set the background color.
    pub const fn set_bg(&mut self, color: Option<Color>) -> &mut Self {
        self.bg_color = color;
        self
    }

    /// Get whether this is bold or not.
    pub const fn bold(&self) -> bool {
        self.bold
    }

    /// Set whether the text is bolded or not.
    pub const fn set_bold(&mut self, yes: bool) -> &mut Self {
        self.bold = yes;
        self
    }

    /// Get whether this is dimmed or not.
    pub const fn dimmed(&self) -> bool {
        self.dimmed
    }

    /// Set whether the text is dimmed or not.
    pub const fn set_dimmed(&mut self, yes: bool) -> &mut Self {
        self.dimmed = yes;
        self
    }

    /// Get whether this is italic or not.
    pub const fn italic(&self) -> bool {
        self.italic
    }

    /// Set whether the text is italicized or not.
    pub const fn set_italic(&mut self, yes: bool) -> &mut Self {
        self.italic = yes;
        self
    }

    /// Get whether this is underline or not.
    pub const fn underline(&self) -> bool {
        self.underline
    }

    /// Set whether the text is underlined or not.
    pub const fn set_underline(&mut self, yes: bool) -> &mut Self {
        self.underline = yes;
        self
    }

    /// Get whether this is strikethrough or not.
    pub const fn strikethrough(&self) -> bool {
        self.strikethrough
    }

    /// Set whether the text is strikethrough or not.
    pub const fn set_strikethrough(&mut self, yes: bool) -> &mut Self {
        self.strikethrough = yes;
        self
    }

    /// Get whether reset is enabled or not.
    ///
    /// reset is enabled by default. When disabled and using ANSI escape
    /// sequences, a "reset" code will be emitted every time a `ColorSpec`'s
    /// settings are applied.
    pub const fn reset(&self) -> bool {
        self.reset
    }

    /// Set whether to reset the terminal whenever color settings are applied.
    ///
    /// reset is enabled by default. When disabled and using ANSI escape
    /// sequences, a "reset" code will be emitted every time a `ColorSpec`'s
    /// settings are applied.
    ///
    /// Typically this is useful if callers have a requirement to more
    /// scrupulously manage the exact sequence of escape codes that are emitted
    /// when using ANSI for colors.
    pub const fn set_reset(&mut self, yes: bool) -> &mut Self {
        self.reset = yes;
        self
    }

    /// Get whether this is intense or not.
    pub const fn intense(&self) -> bool {
        self.intense
    }

    /// Set whether the text is intense or not.
    pub const fn set_intense(&mut self, yes: bool) -> &mut Self {
        self.intense = yes;
        self
    }

    /// Returns true if this color specification has no colors or styles.
    pub const fn is_none(&self) -> bool {
        self.fg_color.is_none()
            && self.bg_color.is_none()
            && !self.bold
            && !self.underline
            && !self.dimmed
            && !self.italic
            && !self.intense
            && !self.strikethrough
    }

    /// Clears this color specification so that it has no color/style settings.
    pub const fn clear(&mut self) {
        self.fg_color = None;
        self.bg_color = None;
        self.bold = false;
        self.underline = false;
        self.intense = false;
        self.dimmed = false;
        self.italic = false;
        self.strikethrough = false;
    }
}

/// The set of available colors for the terminal foreground/background.
///
/// The `Ansi256` and `Rgb` colors will only output the correct codes when
/// paired with the `Ansi` `WriteColor` implementation.
///
/// This set may expand over time.
///
/// This type has a `FromStr` impl that can parse colors from their human
/// readable form. The format is as follows:
///
/// 1. Any of the explicitly listed colors in English. They are matched
///    case insensitively.
/// 2. A single 8-bit integer, in either decimal or hexadecimal format.
/// 3. A triple of 8-bit integers separated by a comma, where each integer is
///    in decimal or hexadecimal format.
///
/// Hexadecimal numbers are written with a `0x` prefix.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Color {
    Black,
    Blue,
    Green,
    Red,
    Cyan,
    Magenta,
    Yellow,
    White,
    Ansi256(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    /// Parses a numeric color string, either ANSI or RGB.
    fn from_str_numeric(s: &str) -> Result<Self, ParseColorError> {
        // The "ansi256" format is a single number (decimal or hex)
        // corresponding to one of 256 colors.
        //
        // The "rgb" format is a triple of numbers (decimal or hex) delimited
        // by a comma corresponding to one of 256^3 colors.

        fn parse_number(s: &str) -> Option<u8> {
            s.strip_prefix("0x")
                .map_or_else(|| s.parse::<u8>(), |s| u8::from_str_radix(s, 16))
                .ok()
        }

        let codes: Vec<&str> = s.split(',').collect();
        if codes.len() == 1 {
            if let Some(n) = parse_number(codes[0]) {
                Ok(Self::Ansi256(n))
            } else if s.chars().all(|c| c.is_ascii_hexdigit()) {
                Err(ParseColorError {
                    kind: ParseColorErrorKind::InvalidAnsi256,
                    given: s.to_string(),
                })
            } else {
                Err(ParseColorError {
                    kind: ParseColorErrorKind::InvalidName,
                    given: s.to_string(),
                })
            }
        } else if codes.len() == 3 {
            let mut v = Vec::new();
            for code in codes {
                let n = parse_number(code).ok_or_else(|| ParseColorError {
                    kind: ParseColorErrorKind::InvalidRgb,
                    given: s.to_string(),
                })?;
                v.push(n);
            }
            Ok(Self::Rgb(v[0], v[1], v[2]))
        } else {
            Err(if s.contains(',') {
                ParseColorError {
                    kind: ParseColorErrorKind::InvalidRgb,
                    given: s.to_string(),
                }
            } else {
                ParseColorError {
                    kind: ParseColorErrorKind::InvalidName,
                    given: s.to_string(),
                }
            })
        }
    }
}

/// An error from parsing an invalid color specification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseColorError {
    kind: ParseColorErrorKind,
    given: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ParseColorErrorKind {
    InvalidName,
    InvalidAnsi256,
    InvalidRgb,
}

impl ParseColorError {
    /// Return the string that couldn't be parsed as a valid color.
    pub fn invalid(&self) -> &str {
        &self.given
    }
}

impl error::Error for ParseColorError {
    fn description(&self) -> &str {
        match self.kind {
            ParseColorErrorKind::InvalidName => "unrecognized color name",
            ParseColorErrorKind::InvalidAnsi256 => {
                "invalid ansi256 color number"
            }
            ParseColorErrorKind::InvalidRgb => "invalid RGB color triple",
        }
    }
}

impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ParseColorErrorKind::InvalidName => write!(
                f,
                "unrecognized color name '{}'. Choose from: \
                 black, blue, green, red, cyan, magenta, yellow, \
                 white",
                self.given
            ),
            ParseColorErrorKind::InvalidAnsi256 => write!(
                f,
                "unrecognized ansi256 color number, \
                 should be '[0-255]' (or a hex number), but is '{}'",
                self.given
            ),
            ParseColorErrorKind::InvalidRgb => write!(
                f,
                "unrecognized RGB color triple, \
                 should be '[0-255],[0-255],[0-255]' (or a hex \
                 triple), but is '{}'",
                self.given
            ),
        }
    }
}

impl FromStr for Color {
    type Err = ParseColorError;

    fn from_str(s: &str) -> Result<Self, ParseColorError> {
        match &*s.to_lowercase() {
            "black" => Ok(Self::Black),
            "blue" => Ok(Self::Blue),
            "green" => Ok(Self::Green),
            "red" => Ok(Self::Red),
            "cyan" => Ok(Self::Cyan),
            "magenta" => Ok(Self::Magenta),
            "yellow" => Ok(Self::Yellow),
            "white" => Ok(Self::White),
            _ => Self::from_str_numeric(s),
        }
    }
}

/// A hyperlink specification.
#[derive(Clone, Debug)]
pub struct HyperlinkSpec<'a> {
    uri: Option<&'a [u8]>,
}

impl<'a> HyperlinkSpec<'a> {
    /// Creates a new hyperlink specification.
    pub const fn open(uri: &'a [u8]) -> Self {
        HyperlinkSpec { uri: Some(uri) }
    }

    /// Creates a hyperlink specification representing no hyperlink.
    pub const fn close() -> Self {
        HyperlinkSpec { uri: None }
    }

    /// Returns the URI of the hyperlink if one is attached to this spec.
    pub const fn uri(&self) -> Option<&'a [u8]> {
        self.uri
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Ansi, Color, ColorSpec, HyperlinkSpec, ParseColorError,
        ParseColorErrorKind, StandardStream, WriteColor,
    };

    fn assert_is_send<T: Send>() {}

    #[test]
    fn standard_stream_is_send() {
        assert_is_send::<StandardStream>();
    }

    #[test]
    fn test_simple_parse_ok() {
        let color = "green".parse::<Color>();
        assert_eq!(color, Ok(Color::Green));
    }

    #[test]
    fn test_256_parse_ok() {
        let color = "7".parse::<Color>();
        assert_eq!(color, Ok(Color::Ansi256(7)));

        let color = "32".parse::<Color>();
        assert_eq!(color, Ok(Color::Ansi256(32)));

        let color = "0xFF".parse::<Color>();
        assert_eq!(color, Ok(Color::Ansi256(0xFF)));
    }

    #[test]
    fn test_256_parse_err_out_of_range() {
        let color = "256".parse::<Color>();
        assert_eq!(
            color,
            Err(ParseColorError {
                kind: ParseColorErrorKind::InvalidAnsi256,
                given: "256".to_string(),
            })
        );
    }

    #[test]
    fn test_rgb_parse_ok() {
        let color = "0,0,0".parse::<Color>();
        assert_eq!(color, Ok(Color::Rgb(0, 0, 0)));

        let color = "0,128,255".parse::<Color>();
        assert_eq!(color, Ok(Color::Rgb(0, 128, 255)));

        let color = "0x0,0x0,0x0".parse::<Color>();
        assert_eq!(color, Ok(Color::Rgb(0, 0, 0)));

        let color = "0x33,0x66,0xFF".parse::<Color>();
        assert_eq!(color, Ok(Color::Rgb(0x33, 0x66, 0xFF)));
    }

    #[test]
    fn test_rgb_parse_err_out_of_range() {
        let color = "0,0,256".parse::<Color>();
        assert_eq!(
            color,
            Err(ParseColorError {
                kind: ParseColorErrorKind::InvalidRgb,
                given: "0,0,256".to_string(),
            })
        );
    }

    #[test]
    fn test_rgb_parse_err_bad_format() {
        let color = "0,0".parse::<Color>();
        assert_eq!(
            color,
            Err(ParseColorError {
                kind: ParseColorErrorKind::InvalidRgb,
                given: "0,0".to_string(),
            })
        );

        let color = "not_a_color".parse::<Color>();
        assert_eq!(
            color,
            Err(ParseColorError {
                kind: ParseColorErrorKind::InvalidName,
                given: "not_a_color".to_string(),
            })
        );
    }

    #[test]
    fn test_var_ansi_write_rgb() {
        let mut buf = Ansi::new(Vec::new());
        let _ = buf.write_color(true, Color::Rgb(254, 253, 255), false);
        assert_eq!(buf.0, b"\x1B[38;2;254;253;255m");
    }

    #[test]
    fn test_reset() {
        let spec = ColorSpec::new();
        let mut buf = Ansi::new(Vec::new());
        buf.set_color(&spec).unwrap();
        assert_eq!(buf.0, b"\x1B[0m");
    }

    #[test]
    fn test_no_reset() {
        let mut spec = ColorSpec::new();
        spec.set_reset(false);

        let mut buf = Ansi::new(Vec::new());
        buf.set_color(&spec).unwrap();
        assert_eq!(buf.0, b"");
    }

    #[test]
    fn test_var_ansi_write_256() {
        let mut buf = Ansi::new(Vec::new());
        let _ = buf.write_color(false, Color::Ansi256(7), false);
        assert_eq!(buf.0, b"\x1B[48;5;7m");

        let mut buf = Ansi::new(Vec::new());
        let _ = buf.write_color(false, Color::Ansi256(208), false);
        assert_eq!(buf.0, b"\x1B[48;5;208m");
    }

    fn all_attributes() -> Vec<ColorSpec> {
        let mut result = Vec::new();
        for fg in [None, Some(Color::Red)] {
            for bg in [None, Some(Color::Red)] {
                for bold in [false, true] {
                    for underline in [false, true] {
                        for intense in [false, true] {
                            for italic in [false, true] {
                                for strikethrough in [false, true] {
                                    for dimmed in [false, true] {
                                        let mut color = ColorSpec::new();
                                        color.set_fg(fg);
                                        color.set_bg(bg);
                                        color.set_bold(bold);
                                        color.set_underline(underline);
                                        color.set_intense(intense);
                                        color.set_italic(italic);
                                        color.set_dimmed(dimmed);
                                        color.set_strikethrough(strikethrough);
                                        result.push(color);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        result
    }

    #[test]
    fn test_is_none() {
        for (i, color) in all_attributes().iter().enumerate() {
            assert_eq!(
                i == 0,
                color.is_none(),
                "{:?} => {}",
                color,
                color.is_none()
            );
        }
    }

    #[test]
    fn test_clear() {
        for color in all_attributes() {
            let mut color1 = color.clone();
            color1.clear();
            assert!(color1.is_none(), "{color:?} => {color1:?}");
        }
    }

    #[test]
    fn test_ansi_hyperlink() {
        let mut buf = Ansi::new(Vec::new());
        buf.set_hyperlink(&HyperlinkSpec::open(b"https://example.com"))
            .unwrap();
        buf.write_str("label").unwrap();
        buf.set_hyperlink(&HyperlinkSpec::close()).unwrap();

        assert_eq!(
            buf.0,
            b"\x1B]8;;https://example.com\x1B\\label\x1B]8;;\x1B\\".to_vec()
        );
    }
}
