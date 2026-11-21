import { Footer } from '../components/Footer.tsx';
import { Head } from '../components/Head.tsx';
import { Nav } from '../components/Nav.tsx';
import { Hero } from '../sections/Hero.tsx';
import { Install } from '../sections/Install.tsx';

export function Landing() {
	return (
		<html lang="en" data-theme="dark">
			<Head />
			<body>
				<Nav />
				<main>
					<Hero />
					<Install />
				</main>
				<Footer />
			</body>
		</html>
	);
}
